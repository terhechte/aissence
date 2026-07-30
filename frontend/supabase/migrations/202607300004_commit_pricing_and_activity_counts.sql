alter table public.commits
  add column input_cost_usd numeric(20, 8)
    check (input_cost_usd is null or input_cost_usd >= 0),
  add column output_cost_usd numeric(20, 8)
    check (output_cost_usd is null or output_cost_usd >= 0),
  add column potential_cost_usd numeric(20, 8)
    check (potential_cost_usd is null or potential_cost_usd >= 0),
  add column pricing_model_ids jsonb not null default '[]'::jsonb
    check (jsonb_typeof(pricing_model_ids) = 'array'),
  add column pricing_calculated_at timestamptz;

alter table public.messages
  add column changed_file_count integer not null default 0
    check (changed_file_count >= 0);

update public.messages m
set changed_file_count = (
  select count(distinct c.file_path)::integer
  from public.changes c
  where c.message_id = m.id
)
where m.kind = 'AiChanged';

create or replace function public.replace_project_snapshot(
  p_project_id uuid,
  p_import_id uuid,
  p_checksum text,
  p_source_version integer,
  p_snapshot jsonb
)
returns jsonb
language plpgsql
security definer
set search_path = public
as $$
declare
  v_owner uuid;
  v_commit jsonb;
  v_agent jsonb;
  v_message jsonb;
  v_change jsonb;
  v_commit_id uuid;
  v_message_id uuid;
  v_commit_order integer := 0;
  v_agent_order integer;
  v_message_order integer;
  v_change_order integer;
  v_changed_file_count integer;
begin
  select owner_id into v_owner from public.projects where id = p_project_id for update;
  if v_owner is null or v_owner <> auth.uid() then
    raise exception 'project not found or not owned by caller';
  end if;

  if exists (
    select 1 from public.projects
    where id = p_project_id and import_checksum = p_checksum
  ) then
    for v_commit in select value from jsonb_array_elements(p_snapshot->'commits')
    loop
      update public.commits
      set
        input_cost_usd = (v_commit->'pricing'->>'input_cost_usd')::numeric,
        output_cost_usd = (v_commit->'pricing'->>'output_cost_usd')::numeric,
        potential_cost_usd = (v_commit->'pricing'->>'potential_cost_usd')::numeric,
        pricing_model_ids = coalesce(v_commit->'pricing'->'model_ids', '[]'::jsonb),
        pricing_calculated_at = (v_commit->'pricing'->>'priced_at')::timestamptz
      where project_id = p_project_id and commit_hash = v_commit->>'hash'
      returning id into v_commit_id;

      v_message_order := 0;
      for v_message in select value from jsonb_array_elements(v_commit->'summary'->'messages')
      loop
        select count(distinct change->>'file')::integer
        into v_changed_file_count
        from jsonb_array_elements(coalesce(v_message->'changes', '[]'::jsonb)) change;

        update public.messages
        set changed_file_count = coalesce(v_changed_file_count, 0)
        where commit_id = v_commit_id and sort_order = v_message_order;
        v_message_order := v_message_order + 1;
      end loop;
    end loop;

    update public.imports
      set status = 'noop', commit_count = jsonb_array_length(p_snapshot->'commits'),
          completed_at = now()
      where id = p_import_id and project_id = p_project_id;
    return jsonb_build_object('ok', true, 'noop', true);
  end if;

  begin
    delete from public.commits where project_id = p_project_id;

    for v_commit in select value from jsonb_array_elements(p_snapshot->'commits')
    loop
      v_commit_id := gen_random_uuid();
      insert into public.commits (
        id, project_id, commit_hash, title, committed_at,
        files_changed, lines_added, lines_deleted,
        user_requested, ai_answered, ai_implemented, meta,
        token_input, token_output, token_cache_read, token_cache_create,
        input_cost_usd, output_cost_usd, potential_cost_usd,
        pricing_model_ids, pricing_calculated_at, sort_order
      ) values (
        v_commit_id, p_project_id, v_commit->>'hash',
        nullif(v_commit->>'title', ''),
        nullif(v_commit->>'date', '')::timestamptz,
        (v_commit->'stats'->>'files_changed')::bigint,
        (v_commit->'stats'->>'lines_added')::bigint,
        (v_commit->'stats'->>'lines_deleted')::bigint,
        v_commit->'summary'->>'user_requested',
        v_commit->'summary'->>'ai_answered',
        v_commit->'summary'->>'ai_implemented',
        coalesce(v_commit->'summary'->>'meta', ''),
        (v_commit->'summary'->'tokens'->>'input')::numeric,
        (v_commit->'summary'->'tokens'->>'output')::numeric,
        (v_commit->'summary'->'tokens'->>'cache_read')::numeric,
        (v_commit->'summary'->'tokens'->>'cache_create')::numeric,
        (v_commit->'pricing'->>'input_cost_usd')::numeric,
        (v_commit->'pricing'->>'output_cost_usd')::numeric,
        (v_commit->'pricing'->>'potential_cost_usd')::numeric,
        coalesce(v_commit->'pricing'->'model_ids', '[]'::jsonb),
        (v_commit->'pricing'->>'priced_at')::timestamptz,
        v_commit_order
      );

      v_agent_order := 0;
      for v_agent in select value from jsonb_array_elements(v_commit->'summary'->'agents')
      loop
        insert into public.commit_agents (commit_id, provider, model, sort_order)
        values (v_commit_id, v_agent->>'provider', v_agent->>'model', v_agent_order);
        v_agent_order := v_agent_order + 1;
      end loop;

      v_message_order := 0;
      for v_message in select value from jsonb_array_elements(v_commit->'summary'->'messages')
      loop
        v_message_id := gen_random_uuid();
        select count(distinct change->>'file')::integer
        into v_changed_file_count
        from jsonb_array_elements(coalesce(v_message->'changes', '[]'::jsonb)) change;

        insert into public.messages (
          id, commit_id, kind, summary, occurred_at,
          token_input, token_output, token_cache_read, token_cache_create,
          changed_file_count, sort_order
        ) values (
          v_message_id, v_commit_id, (v_message->>'kind')::public.message_kind,
          v_message->>'summary', nullif(v_message->>'timestamp', '')::timestamptz,
          (v_message->'tokens'->>'input')::numeric,
          (v_message->'tokens'->>'output')::numeric,
          (v_message->'tokens'->>'cache_read')::numeric,
          (v_message->'tokens'->>'cache_create')::numeric,
          coalesce(v_changed_file_count, 0),
          v_message_order
        );

        if v_message ? 'content' and v_message->>'content' is not null then
          insert into public.message_sources (message_id, content)
          values (v_message_id, v_message->>'content');
        end if;

        v_change_order := 0;
        for v_change in select value from jsonb_array_elements(v_message->'changes')
        loop
          insert into public.changes (message_id, tool, file_path, diff, sort_order)
          values (
            v_message_id, v_change->>'tool', v_change->>'file',
            v_change->>'diff', v_change_order
          );
          v_change_order := v_change_order + 1;
        end loop;
        v_message_order := v_message_order + 1;
      end loop;
      v_commit_order := v_commit_order + 1;
    end loop;

    update public.projects
      set source_version = p_source_version, import_checksum = p_checksum
      where id = p_project_id;
    update public.imports
      set status = 'succeeded', commit_count = v_commit_order, completed_at = now()
      where id = p_import_id and project_id = p_project_id;

    return jsonb_build_object('ok', true, 'noop', false, 'commit_count', v_commit_order);
  exception when others then
    update public.imports
      set status = 'failed', error_detail = left(sqlerrm, 1000), completed_at = now()
      where id = p_import_id and project_id = p_project_id;
    return jsonb_build_object('ok', false, 'error', left(sqlerrm, 1000));
  end;
end;
$$;

revoke all on function public.replace_project_snapshot(uuid, uuid, text, integer, jsonb)
from public;
grant execute on function public.replace_project_snapshot(uuid, uuid, text, integer, jsonb)
to authenticated;
