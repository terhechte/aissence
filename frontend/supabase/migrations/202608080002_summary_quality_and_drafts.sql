alter table public.commits
    add column if not exists summary_origin text not null default 'pending';

alter table public.commits
    drop constraint if exists commits_summary_origin_check;
alter table public.commits
    add constraint commits_summary_origin_check check (
      summary_origin in ('pending','model','metadata_fallback','parse_fallback','privacy_fallback')
    );

update public.commits
set summary_origin = case when btrim(public_story) = '' then 'pending' else 'model' end
where summary_origin = 'pending';

-- Version 2 reserves grounding evidence, removes transport noise, generates
-- the compact subline in the public-story request, and never lets subscriber
-- work rewrite public copy. Outdated or incomplete stories are eligible both
-- for private creator preparation and the normal publish trigger.
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
      or btrim(c.public_subline) = ''
      or coalesce(c.summarizer_version, 0) < 2
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
      or btrim(public_subline) = ''
      or coalesce(summarizer_version, 0) < 2
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
        and ms.summarizer_version >= 2
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

-- Published ledgers are refreshed automatically. Drafts remain private and
-- are queued only through the creator's explicit Generate summaries action.
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
  and coalesce(c.summarizer_version, 0) < 2
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
  and coalesce(c.summarizer_version, 0) < 2;

-- Directory and creator cards reuse the newest successful commit summary.
-- DISTINCT ON runs in Postgres rather than downloading every commit and avoids
-- the API row cap biasing large project collections toward whichever projects
-- happen to occur first.
create or replace view public.project_summary_overviews
with (security_invoker = true)
as
select distinct on (c.project_id)
  c.project_id,
  c.title,
  c.public_subline,
  c.public_story,
  c.sort_order
from public.commits c
where c.summary_status = 'succeeded'
  and coalesce(c.summarizer_version, 0) >= 2
  and (btrim(c.public_subline) <> '' or btrim(c.public_story) <> '')
order by c.project_id, c.sort_order desc;

grant select on public.project_summary_overviews to anon, authenticated, service_role;
