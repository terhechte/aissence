do $$ begin
    create type public.summary_job_kind as enum ('public', 'subscriber');
exception when duplicate_object then null;
end $$;

alter table public.commits
    add column if not exists public_story text not null default '',
    add column if not exists public_beats jsonb not null default '[]'::jsonb,
    add column if not exists prompt_count integer not null default 0,
    add column if not exists reply_count integer not null default 0,
    add column if not exists change_message_count integer not null default 0;

alter table public.commits
    drop constraint if exists commits_public_beats_check;
alter table public.commits
    add constraint commits_public_beats_check check (
        jsonb_typeof(public_beats) = 'array' and jsonb_array_length(public_beats) <= 3
    );

create table if not exists public.message_summaries (
    message_id uuid primary key references public.messages(id) on delete cascade,
    summary text not null check (btrim(summary) <> ''),
    summarizer_version integer not null,
    generated_at timestamptz not null default now()
);

create table if not exists public.summary_jobs (
    id uuid primary key default gen_random_uuid(),
    import_id uuid not null references public.imports(id) on delete cascade,
    commit_id uuid not null references public.commits(id) on delete cascade,
    kind public.summary_job_kind not null,
    status public.summary_job_status not null default 'queued',
    attempts integer not null default 0 check (attempts >= 0),
    payload jsonb not null,
    error_detail text,
    lease_expires_at timestamptz,
    created_at timestamptz not null default now(),
    started_at timestamptz,
    completed_at timestamptz,
    unique (commit_id, kind)
);

create index if not exists summary_jobs_queue_idx
    on public.summary_jobs(status, lease_expires_at, created_at);

alter table public.summarization_attempts
    add column if not exists summary_job_id uuid references public.summary_jobs(id) on delete set null;

-- Public readers get commit-level stories and deterministic counts only. Raw
-- message rows, prompt sources, subscriber summaries, and changes all require
-- an owner or active subscriber as appropriate.
drop policy if exists "messages follow commit visibility" on public.messages;
drop policy if exists "source is owner or entitled user prompt" on public.message_sources;
drop policy if exists "messages are owner or entitled" on public.messages;
drop policy if exists "subscriber summaries are owner or entitled" on public.message_summaries;
drop policy if exists "owners inspect summary jobs" on public.summary_jobs;

create policy "messages are owner or entitled" on public.messages for select
using (exists (
    select 1
    from public.commits c
    join public.projects p on p.id = c.project_id
    where c.id = commit_id
      and (
        p.owner_id = auth.uid()
        or (p.visibility = 'published' and public.has_creator_access(p.owner_id))
      )
));

create policy "source is owner or entitled user prompt" on public.message_sources for select
using (exists (
    select 1
    from public.messages m
    join public.commits c on c.id = m.commit_id
    join public.projects p on p.id = c.project_id
    where m.id = message_id
      and (
        p.owner_id = auth.uid()
        or (
          p.visibility = 'published'
          and m.kind = 'UserWrote'
          and public.has_creator_access(p.owner_id)
        )
      )
));

alter table public.message_summaries enable row level security;
create policy "subscriber summaries are owner or entitled" on public.message_summaries for select
using (exists (
    select 1
    from public.messages m
    join public.commits c on c.id = m.commit_id
    join public.projects p on p.id = c.project_id
    where m.id = message_id
      and (
        p.owner_id = auth.uid()
        or (p.visibility = 'published' and public.has_creator_access(p.owner_id))
      )
));

alter table public.summary_jobs enable row level security;
create policy "owners inspect summary jobs" on public.summary_jobs for select to authenticated
using (exists (
    select 1 from public.commits c
    where c.id = commit_id and public.is_project_owner(c.project_id)
));

-- Remove the raw-content preview and the audience-ambiguous summary column.
-- The tables are unreleased, so no compatibility view is retained.
alter table public.messages drop column if exists preview;
alter table public.messages drop column if exists summary;

alter table public.commits
    drop column if exists user_requested,
    drop column if exists ai_answered,
    drop column if exists ai_implemented,
    drop column if exists meta;

drop function if exists public.replace_project_snapshot(uuid, uuid, text, integer, jsonb);
drop function if exists public.message_preview(text);

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
    and btrim(c.public_story) = ''
    and c.prompt_count > 0
  on conflict (commit_id, kind) do update
    set import_id = excluded.import_id,
        payload = excluded.payload,
        status = 'queued',
        error_detail = null,
        lease_expires_at = null,
        completed_at = null;
  get diagnostics v_count = row_count;
  update public.commits
  set summary_status = 'queued', summary_error = null
  where project_id = p_project_id
    and btrim(public_story) = ''
    and prompt_count > 0;
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
      where m.commit_id = c.id and m.kind = 'UserWrote'
    )
  on conflict (commit_id, kind) do update
    set import_id = excluded.import_id,
        payload = excluded.payload,
        status = 'queued',
        error_detail = null,
        lease_expires_at = null,
        completed_at = null;
  get diagnostics v_count = row_count;
  return v_count;
end;
$$;

revoke all on function public.enqueue_subscriber_summary_jobs(uuid) from public;
grant execute on function public.enqueue_subscriber_summary_jobs(uuid) to service_role;

create or replace function public.queue_public_stories_after_publish()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  if new.visibility = 'published' and old.visibility is distinct from new.visibility then
    perform public.enqueue_public_summary_jobs(new.id);
  end if;
  return new;
end;
$$;

drop trigger if exists projects_queue_public_stories on public.projects;
create trigger projects_queue_public_stories
after update of visibility on public.projects
for each row execute function public.queue_public_stories_after_publish();

create or replace function public.queue_subscriber_summaries_after_access()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  if new.status in ('active', 'trialing')
     and (new.current_period_end is null or new.current_period_end > now()) then
    perform public.enqueue_subscriber_summary_jobs(new.creator_id);
  end if;
  return new;
end;
$$;

drop trigger if exists subscriptions_queue_prompt_summaries on public.creator_subscriptions;
create trigger subscriptions_queue_prompt_summaries
after insert or update of status, current_period_end on public.creator_subscriptions
for each row execute function public.queue_subscriber_summaries_after_access();
