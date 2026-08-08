alter table public.commits
    add column if not exists public_subline text not null default '';

alter table public.commits
    drop constraint if exists commits_public_subline_check;
alter table public.commits
    add constraint commits_public_subline_check check (char_length(public_subline) <= 1000);

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
    and (btrim(c.public_story) = '' or btrim(c.public_subline) = '')
    and c.prompt_count > 0
  on conflict (commit_id, kind) do update
    set import_id = excluded.import_id,
        payload = excluded.payload,
        status = 'queued',
        attempts = 0,
        error_detail = null,
        lease_expires_at = null,
        started_at = null,
        completed_at = null;
  get diagnostics v_count = row_count;
  update public.commits
  set summary_status = 'queued', summary_error = null
  where project_id = p_project_id
    and (btrim(public_story) = '' or btrim(public_subline) = '')
    and prompt_count > 0;
  return v_count;
end;
$$;

revoke all on function public.enqueue_public_summary_jobs(uuid) from public;
grant execute on function public.enqueue_public_summary_jobs(uuid) to authenticated, service_role;

-- Backfill published commits so existing ledgers receive sublines after deploy.
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
  and btrim(c.public_subline) = ''
on conflict (commit_id, kind) do update
  set import_id = excluded.import_id,
      payload = excluded.payload,
      status = 'queued',
      attempts = 0,
      error_detail = null,
      lease_expires_at = null,
      started_at = null,
      completed_at = null
  where public.summary_jobs.status = 'succeeded';

update public.commits c
set summary_status = 'queued', summary_error = null
from public.projects p
where p.id = c.project_id
  and p.visibility = 'published'
  and c.prompt_count > 0
  and btrim(c.public_subline) = ''
  and c.summary_status = 'succeeded';
