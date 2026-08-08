-- Publish only after acquiring the same project-row lock used by ingestion.
-- The subsequent readiness queries therefore observe any import that was
-- already in flight before the publish attempt.
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
      or coalesce(summarizer_version, 0) < 2
      or btrim(public_story) = ''
      or btrim(public_subline) = ''
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

-- This is a final database-level guard for direct table updates. The RPC above
-- supplies the stronger concurrency guarantee by locking before it checks.
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
          or coalesce(summarizer_version, 0) < 2
          or btrim(public_story) = ''
          or btrim(public_subline) = ''
        )
    ) then
      raise exception 'Generate and review all summaries before publishing.';
    end if;
  end if;
  return new;
end;
$$;

drop trigger if exists projects_require_ready_summaries on public.projects;
create trigger projects_require_ready_summaries
before update of visibility on public.projects
for each row execute function public.enforce_project_publish_readiness();
