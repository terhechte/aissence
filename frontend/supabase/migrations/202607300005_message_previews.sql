create or replace function public.message_preview(p_content text)
returns text
language sql
immutable
set search_path = public
as $$
  select case
    when p_content is null then null
    when char_length(p_content) <= 320 then p_content
    else rtrim(left(p_content, 319)) || '…'
  end;
$$;

alter table public.messages add column if not exists preview text;
alter table public.messages alter column summary drop not null;

update public.messages m
set preview = public.message_preview(s.content)
from public.message_sources s
where s.message_id = m.id and m.preview is null;

do $$
declare
  v_definition text;
begin
  select pg_get_functiondef(
    'public.replace_project_snapshot(uuid,uuid,text,integer,jsonb)'::regprocedure
  ) into v_definition;
  v_definition := replace(
    v_definition,
    'id, commit_id, kind, summary, occurred_at,',
    'id, commit_id, kind, summary, preview, occurred_at,'
  );
  v_definition := replace(
    v_definition,
    'v_message->>''summary'', nullif(v_message->>''timestamp'', '''')::timestamptz,',
    'v_message->>''summary'', public.message_preview(v_message->>''content''), nullif(v_message->>''timestamp'', '''')::timestamptz,'
  );
  execute v_definition;
end;
$$;
