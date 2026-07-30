create extension if not exists pgcrypto;
create extension if not exists citext;

create type public.project_visibility as enum ('draft', 'published');
create type public.import_status as enum ('processing', 'succeeded', 'failed', 'noop');
create type public.message_kind as enum ('UserWrote', 'AiWrote', 'AiChanged');
create type public.subscription_status as enum ('active', 'trialing', 'past_due', 'canceled');
create type public.subscription_source as enum ('manual', 'stripe');

create table public.profiles (
  id uuid primary key references auth.users(id) on delete cascade,
  handle citext not null unique check (handle::text ~ '^[a-z0-9][a-z0-9-]{2,39}$'),
  display_name text not null check (char_length(display_name) between 1 and 100),
  bio text check (bio is null or char_length(bio) <= 500),
  avatar_path text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table public.projects (
  id uuid primary key default gen_random_uuid(),
  owner_id uuid not null references public.profiles(id) on delete cascade,
  slug citext not null check (slug::text ~ '^[a-z0-9][a-z0-9-]{1,79}$'),
  name text not null check (char_length(name) between 1 and 120),
  tagline text not null check (char_length(tagline) between 1 and 240),
  description text not null check (char_length(description) between 1 and 3000),
  product_url text not null check (product_url ~ '^https://'),
  repository_label text not null check (char_length(repository_label) between 1 and 160),
  repository_url text check (repository_url is null or repository_url ~ '^https://'),
  visibility public.project_visibility not null default 'draft',
  source_version integer,
  import_checksum text,
  published_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (owner_id, slug)
);

create table public.project_screenshots (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references public.projects(id) on delete cascade,
  storage_path text not null unique,
  alt_text text not null check (char_length(alt_text) between 1 and 300),
  caption text check (caption is null or char_length(caption) <= 300),
  sort_order integer not null check (sort_order between -1 and 7),
  created_at timestamptz not null default now(),
  unique (project_id, sort_order)
);

create table public.imports (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references public.projects(id) on delete cascade,
  uploader_id uuid not null references auth.users(id) on delete cascade,
  checksum text not null,
  source_version integer not null,
  status public.import_status not null default 'processing',
  commit_count integer,
  warnings jsonb not null default '[]'::jsonb,
  error_detail text,
  created_at timestamptz not null default now(),
  completed_at timestamptz
);

create table public.commits (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references public.projects(id) on delete cascade,
  commit_hash text not null check (commit_hash ~ '^[0-9a-f]{40}$'),
  title text,
  committed_at timestamptz,
  files_changed bigint check (files_changed is null or files_changed >= 0),
  lines_added bigint check (lines_added is null or lines_added >= 0),
  lines_deleted bigint check (lines_deleted is null or lines_deleted >= 0),
  user_requested text not null,
  ai_answered text not null,
  ai_implemented text not null,
  meta text not null default '',
  token_input numeric(20,0),
  token_output numeric(20,0),
  token_cache_read numeric(20,0),
  token_cache_create numeric(20,0),
  sort_order integer not null,
  unique (project_id, commit_hash)
);

create table public.commit_agents (
  id uuid primary key default gen_random_uuid(),
  commit_id uuid not null references public.commits(id) on delete cascade,
  provider text not null,
  model text,
  sort_order integer not null,
  unique (commit_id, sort_order)
);

create table public.messages (
  id uuid primary key default gen_random_uuid(),
  commit_id uuid not null references public.commits(id) on delete cascade,
  kind public.message_kind not null,
  summary text,
  preview text,
  occurred_at timestamptz,
  token_input numeric(20,0),
  token_output numeric(20,0),
  token_cache_read numeric(20,0),
  token_cache_create numeric(20,0),
  sort_order integer not null,
  unique (commit_id, sort_order)
);

create table public.message_sources (
  message_id uuid primary key references public.messages(id) on delete cascade,
  content text not null
);

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

create table public.changes (
  id uuid primary key default gen_random_uuid(),
  message_id uuid not null references public.messages(id) on delete cascade,
  tool text not null,
  file_path text not null,
  diff text not null,
  sort_order integer not null,
  unique (message_id, sort_order)
);

create table public.creator_subscriptions (
  id uuid primary key default gen_random_uuid(),
  subscriber_id uuid not null references auth.users(id) on delete cascade,
  creator_id uuid not null references public.profiles(id) on delete cascade,
  status public.subscription_status not null,
  source public.subscription_source not null default 'manual',
  external_customer_id text,
  external_subscription_id text,
  current_period_end timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (subscriber_id, creator_id),
  check (subscriber_id <> creator_id)
);

create index projects_public_idx on public.projects (published_at desc) where visibility = 'published';
create index commits_project_date_idx on public.commits (project_id, committed_at, sort_order);
create index messages_commit_order_idx on public.messages (commit_id, sort_order);
create index subscriptions_access_idx on public.creator_subscriptions (subscriber_id, creator_id, status);

create or replace function public.touch_updated_at()
returns trigger language plpgsql as $$
begin
  new.updated_at = now();
  return new;
end;
$$;

create trigger profiles_touch_updated_at before update on public.profiles
for each row execute function public.touch_updated_at();
create trigger projects_touch_updated_at before update on public.projects
for each row execute function public.touch_updated_at();
create trigger subscriptions_touch_updated_at before update on public.creator_subscriptions
for each row execute function public.touch_updated_at();

create or replace function public.is_project_owner(p_project_id uuid)
returns boolean
language sql stable security definer
set search_path = public
as $$
  select exists (
    select 1 from public.projects p
    where p.id = p_project_id and p.owner_id = auth.uid()
  );
$$;

create or replace function public.can_view_project(p_project_id uuid)
returns boolean
language sql stable security definer
set search_path = public
as $$
  select exists (
    select 1 from public.projects p
    where p.id = p_project_id
      and (p.visibility = 'published' or p.owner_id = auth.uid())
  );
$$;

create or replace function public.has_creator_access(p_creator_id uuid)
returns boolean
language sql stable security definer
set search_path = public
as $$
  select auth.uid() = p_creator_id or exists (
    select 1
    from public.creator_subscriptions s
    where s.subscriber_id = auth.uid()
      and s.creator_id = p_creator_id
      and s.status in ('active', 'trialing')
      and (s.current_period_end is null or s.current_period_end > now())
  );
$$;

alter table public.profiles enable row level security;
alter table public.projects enable row level security;
alter table public.project_screenshots enable row level security;
alter table public.imports enable row level security;
alter table public.commits enable row level security;
alter table public.commit_agents enable row level security;
alter table public.messages enable row level security;
alter table public.message_sources enable row level security;
alter table public.changes enable row level security;
alter table public.creator_subscriptions enable row level security;

create policy "profiles are public" on public.profiles for select using (true);
create policy "users create their profile" on public.profiles for insert to authenticated
with check (id = auth.uid());
create policy "users update their profile" on public.profiles for update to authenticated
using (id = auth.uid()) with check (id = auth.uid());

create policy "published projects or own drafts are visible" on public.projects for select
using (visibility = 'published' or owner_id = auth.uid());
create policy "creators create projects" on public.projects for insert to authenticated
with check (owner_id = auth.uid());
create policy "creators update projects" on public.projects for update to authenticated
using (owner_id = auth.uid()) with check (owner_id = auth.uid());
create policy "creators delete projects" on public.projects for delete to authenticated
using (owner_id = auth.uid());

create policy "screenshots follow project visibility" on public.project_screenshots for select
using (public.can_view_project(project_id));
create policy "owners add screenshots" on public.project_screenshots for insert to authenticated
with check (public.is_project_owner(project_id));
create policy "owners update screenshots" on public.project_screenshots for update to authenticated
using (public.is_project_owner(project_id)) with check (public.is_project_owner(project_id));
create policy "owners delete screenshots" on public.project_screenshots for delete to authenticated
using (public.is_project_owner(project_id));

create policy "owners see imports" on public.imports for select to authenticated
using (public.is_project_owner(project_id));
create policy "owners create imports" on public.imports for insert to authenticated
with check (uploader_id = auth.uid() and public.is_project_owner(project_id));
create policy "owners update imports" on public.imports for update to authenticated
using (uploader_id = auth.uid() and public.is_project_owner(project_id));

create policy "commits follow project visibility" on public.commits for select
using (public.can_view_project(project_id));
create policy "agents follow commit visibility" on public.commit_agents for select
using (exists (
  select 1 from public.commits c where c.id = commit_id and public.can_view_project(c.project_id)
));
create policy "messages follow commit visibility" on public.messages for select
using (exists (
  select 1 from public.commits c where c.id = commit_id and public.can_view_project(c.project_id)
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

create policy "changes are creator only" on public.changes for select to authenticated
using (exists (
  select 1
  from public.messages m
  join public.commits c on c.id = m.commit_id
  join public.projects p on p.id = c.project_id
  where m.id = message_id and p.owner_id = auth.uid()
));

create policy "subscription parties can read" on public.creator_subscriptions for select to authenticated
using (subscriber_id = auth.uid() or creator_id = auth.uid());

insert into storage.buckets (id, name, public, file_size_limit)
values
  ('aissence-staging', 'aissence-staging', false, 26214400),
  ('project-media', 'project-media', false, 8388608)
on conflict (id) do nothing;

create policy "owners upload staging files" on storage.objects for insert to authenticated
with check (
  bucket_id = 'aissence-staging'
  and (storage.foldername(name))[1] = auth.uid()::text
);
create policy "owners read staging files" on storage.objects for select to authenticated
using (
  bucket_id = 'aissence-staging'
  and (storage.foldername(name))[1] = auth.uid()::text
);
create policy "owners delete staging files" on storage.objects for delete to authenticated
using (
  bucket_id = 'aissence-staging'
  and (storage.foldername(name))[1] = auth.uid()::text
);

create policy "owners upload project media" on storage.objects for insert to authenticated
with check (
  bucket_id = 'project-media'
  and (storage.foldername(name))[1] = auth.uid()::text
  and exists (
    select 1 from public.projects p
    where p.owner_id = auth.uid()
      and p.id::text = (storage.foldername(name))[2]
  )
);
create policy "project media follows project visibility" on storage.objects for select
using (
  bucket_id = 'project-media'
  and exists (
    select 1 from public.projects p
    where p.id::text = (storage.foldername(name))[2]
      and (p.visibility = 'published' or p.owner_id = auth.uid())
  )
);
create policy "owners delete project media" on storage.objects for delete to authenticated
using (
  bucket_id = 'project-media'
  and (storage.foldername(name))[1] = auth.uid()::text
);

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
begin
  select owner_id into v_owner from public.projects where id = p_project_id for update;
  if v_owner is null or v_owner <> auth.uid() then
    raise exception 'project not found or not owned by caller';
  end if;

  if exists (
    select 1 from public.projects
    where id = p_project_id and import_checksum = p_checksum
  ) then
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
        token_input, token_output, token_cache_read, token_cache_create, sort_order
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
        insert into public.messages (
          id, commit_id, kind, summary, preview, occurred_at,
          token_input, token_output, token_cache_read, token_cache_create, sort_order
        ) values (
          v_message_id, v_commit_id, (v_message->>'kind')::public.message_kind,
          v_message->>'summary', public.message_preview(v_message->>'content'),
          nullif(v_message->>'timestamp', '')::timestamptz,
          (v_message->'tokens'->>'input')::numeric,
          (v_message->'tokens'->>'output')::numeric,
          (v_message->'tokens'->>'cache_read')::numeric,
          (v_message->'tokens'->>'cache_create')::numeric,
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

revoke all on function public.replace_project_snapshot(uuid, uuid, text, integer, jsonb) from public;
grant execute on function public.replace_project_snapshot(uuid, uuid, text, integer, jsonb) to authenticated;

create or replace function public.grant_manual_subscription(
  p_subscriber_id uuid,
  p_creator_id uuid,
  p_expires_at timestamptz default null
)
returns uuid
language plpgsql
security definer
set search_path = public
as $$
declare
  v_id uuid;
begin
  if auth.role() <> 'service_role' then
    raise exception 'service role required';
  end if;
  insert into public.creator_subscriptions (
    subscriber_id, creator_id, status, source, current_period_end
  ) values (
    p_subscriber_id, p_creator_id, 'active', 'manual', p_expires_at
  )
  on conflict (subscriber_id, creator_id) do update
    set status = 'active', source = 'manual',
        current_period_end = excluded.current_period_end, updated_at = now()
  returning id into v_id;
  return v_id;
end;
$$;

revoke all on function public.grant_manual_subscription(uuid, uuid, timestamptz) from public;
grant execute on function public.grant_manual_subscription(uuid, uuid, timestamptz) to service_role;

create or replace function public.move_project_screenshot(
  p_screenshot_id uuid,
  p_direction integer
)
returns void
language plpgsql
security definer
set search_path = public
as $$
declare
  v_project_id uuid;
  v_current integer;
  v_target integer;
  v_other uuid;
begin
  select project_id, sort_order into v_project_id, v_current
  from public.project_screenshots where id = p_screenshot_id for update;
  if v_project_id is null or not public.is_project_owner(v_project_id) then
    raise exception 'screenshot not found or not owned by caller';
  end if;
  v_target := v_current + case when p_direction < 0 then -1 else 1 end;
  select id into v_other from public.project_screenshots
    where project_id = v_project_id and sort_order = v_target for update;
  if v_other is null then return; end if;
  update public.project_screenshots set sort_order = -1 where id = p_screenshot_id;
  update public.project_screenshots set sort_order = v_current where id = v_other;
  update public.project_screenshots set sort_order = v_target where id = p_screenshot_id;
end;
$$;

revoke all on function public.move_project_screenshot(uuid, integer) from public;
grant execute on function public.move_project_screenshot(uuid, integer) to authenticated;
