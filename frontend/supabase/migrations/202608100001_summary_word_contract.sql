-- Version 4 standardizes summary lengths, introduces explicit model failover,
-- and synthesizes public stories from privacy-safe prompt summaries. Requeue
-- older published copy and make version 4 the readiness threshold without
-- requiring the retired public_subline field.
create or replace function public.enqueue_public_summary_jobs(p_project_id uuid)
returns integer
language plpgsql
security definer
set search_path = public
as $$
declare
  v_owner uuid;
  v_count integer;
begin
  select owner_id into v_owner from public.projects where id = p_project_id;
  if v_owner is null then
    raise exception 'project not found';
  end if;
  if auth.uid() is not null and auth.uid() <> v_owner then
    raise exception 'project not owned by caller';
  end if;

  insert into public.summary_jobs (import_id, commit_id, kind, payload)
  select latest.import_id, c.id, 'public', latest.payload
  from public.commits c
  join lateral (
    select ic.import_id, ic.payload
    from public.import_commits ic
    where ic.commit_id = c.id
    order by ic.created_at desc
    limit 1
  ) latest on true
  where c.project_id = p_project_id
    and c.prompt_count > 0
    and (
      btrim(c.public_story) = ''
      or coalesce(c.summarizer_version, 0) < 5
    )
  on conflict (commit_id, kind) do update
    set import_id = excluded.import_id,
        payload = excluded.payload,
        status = 'queued',
        attempts = 0,
        error_detail = null,
        lease_expires_at = null,
        started_at = null,
        completed_at = null
    where public.summary_jobs.status not in ('queued', 'processing');
  get diagnostics v_count = row_count;

  update public.commits
  set summary_status = 'queued', summary_origin = 'pending', summary_error = null
  where project_id = p_project_id
    and prompt_count > 0
    and summary_status <> 'processing'
    and (
      btrim(public_story) = ''
      or coalesce(summarizer_version, 0) < 5
    );
  return v_count;
end;
$$;

revoke all on function public.enqueue_public_summary_jobs(uuid) from public;
grant execute on function public.enqueue_public_summary_jobs(uuid) to authenticated, service_role;

create or replace function public.enqueue_subscriber_summary_jobs(p_creator_id uuid)
returns integer
language plpgsql
security definer
set search_path = public
as $$
declare
  v_count integer;
begin
  insert into public.summary_jobs (import_id, commit_id, kind, payload)
  select latest.import_id, c.id, 'subscriber', latest.payload
  from public.commits c
  join public.projects p on p.id = c.project_id
  join lateral (
    select ic.import_id, ic.payload
    from public.import_commits ic
    where ic.commit_id = c.id
    order by ic.created_at desc
    limit 1
  ) latest on true
  where p.owner_id = p_creator_id
    and p.visibility = 'published'
    and c.prompt_count > (
      select count(*)
      from public.messages m
      join public.message_summaries ms on ms.message_id = m.id
      where m.commit_id = c.id
        and m.kind = 'UserWrote'
        and ms.summarizer_version >= 5
    )
  on conflict (commit_id, kind) do update
    set import_id = excluded.import_id,
        payload = excluded.payload,
        status = 'queued',
        attempts = 0,
        error_detail = null,
        lease_expires_at = null,
        started_at = null,
        completed_at = null
    where public.summary_jobs.status not in ('queued', 'processing');
  get diagnostics v_count = row_count;
  return v_count;
end;
$$;

revoke all on function public.enqueue_subscriber_summary_jobs(uuid) from public;
grant execute on function public.enqueue_subscriber_summary_jobs(uuid) to service_role;

create or replace function public.publish_ready_project(p_project_id uuid)
returns void
language plpgsql
security definer
set search_path = public
as $$
declare
  v_owner uuid;
  v_unfinished integer;
begin
  select owner_id into v_owner
  from public.projects
  where id = p_project_id
  for update;

  if v_owner is null then
    raise exception 'project not found';
  end if;
  if auth.uid() is null or auth.uid() <> v_owner then
    raise exception 'project not owned by caller';
  end if;
  if not exists (
    select 1
    from public.imports
    where project_id = p_project_id
      and status in ('succeeded', 'noop')
  ) then
    raise exception 'A successful import is required.';
  end if;

  select count(*)::integer into v_unfinished
  from public.commits
  where project_id = p_project_id
    and prompt_count > 0
    and (
      summary_status is distinct from 'succeeded'
      or coalesce(summarizer_version, 0) < 5
      or btrim(public_story) = ''
    );

  if v_unfinished > 0 then
    raise exception 'Generate and review all summaries before publishing (% remaining).', v_unfinished;
  end if;

  perform set_config('aissence.publish_ready_project', p_project_id::text, true);
  update public.projects
  set visibility = 'published',
      published_at = coalesce(published_at, now())
  where id = p_project_id;
end;
$$;

revoke all on function public.publish_ready_project(uuid) from public;
grant execute on function public.publish_ready_project(uuid) to authenticated;

create or replace function public.enforce_project_publish_readiness()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  if new.visibility = 'published' and old.visibility is distinct from new.visibility then
    if current_setting('aissence.publish_ready_project', true) is distinct from new.id::text then
      raise exception 'Publish projects with publish_ready_project().';
    end if;
    if not exists (
      select 1
      from public.imports
      where project_id = new.id
        and status in ('succeeded', 'noop')
    ) then
      raise exception 'A successful import is required.';
    end if;
    if exists (
      select 1
      from public.commits
      where project_id = new.id
        and prompt_count > 0
        and (
          summary_status is distinct from 'succeeded'
          or coalesce(summarizer_version, 0) < 5
          or btrim(public_story) = ''
        )
    ) then
      raise exception 'Generate and review all summaries before publishing.';
    end if;
  end if;
  return new;
end;
$$;

-- Refresh published public stories immediately. Drafts remain opt-in.
insert into public.summary_jobs (import_id, commit_id, kind, payload)
select latest.import_id, c.id, 'public', latest.payload
from public.commits c
join public.projects p on p.id = c.project_id
join lateral (
  select ic.import_id, ic.payload
  from public.import_commits ic
  where ic.commit_id = c.id
  order by ic.created_at desc
  limit 1
) latest on true
where p.visibility = 'published'
  and c.prompt_count > 0
  and coalesce(c.summarizer_version, 0) < 5
on conflict (commit_id, kind) do update
  set import_id = excluded.import_id,
      payload = excluded.payload,
      status = 'queued',
      attempts = 0,
      error_detail = null,
      lease_expires_at = null,
      started_at = null,
      completed_at = null
  where public.summary_jobs.status not in ('queued', 'processing');

update public.commits c
set summary_status = 'queued', summary_origin = 'pending', summary_error = null
from public.projects p
where p.id = c.project_id
  and p.visibility = 'published'
  and c.prompt_count > 0
  and c.summary_status <> 'processing'
  and coalesce(c.summarizer_version, 0) < 5;

-- Existing active subscriber relationships should also receive version-4
-- prompt summaries without waiting for the next subscription update.
do $$
declare
  subscription record;
begin
  for subscription in
    select distinct creator_id
    from public.creator_subscriptions
    where status in ('active', 'trialing')
      and (current_period_end is null or current_period_end > now())
  loop
    perform public.enqueue_subscriber_summary_jobs(subscription.creator_id);
  end loop;
end;
$$;
