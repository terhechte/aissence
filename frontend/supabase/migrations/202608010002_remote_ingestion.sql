do $$ begin
    create type public.summary_job_status as enum ('queued', 'processing', 'succeeded', 'failed', 'skipped');
exception when duplicate_object then null;
end $$;

alter table public.projects
    add column if not exists source_id uuid,
    add column if not exists sync_revision bigint not null default 0,
    add column if not exists last_imported_at timestamptz;

create unique index if not exists projects_owner_source_id_key
    on public.projects(owner_id, source_id) where source_id is not null;

alter table public.commits
    add column if not exists source_digest text,
    add column if not exists sanitizer_version integer,
    add column if not exists summarizer_version integer,
    add column if not exists summary_status public.summary_job_status,
    add column if not exists summarized_at timestamptz,
    add column if not exists summary_error text;

alter table public.imports
    add column if not exists submitted_commit_count integer not null default 0,
    add column if not exists summarized_commit_count integer not null default 0,
    add column if not exists skipped_commit_count integer not null default 0;

create unique index if not exists imports_project_checksum_key on public.imports(project_id, checksum);

create table if not exists public.import_commits (
    import_id uuid not null references public.imports(id) on delete cascade,
    commit_id uuid not null references public.commits(id) on delete cascade,
    status public.summary_job_status not null default 'queued',
    attempts integer not null default 0 check (attempts >= 0),
    payload jsonb,
    error_detail text,
    lease_expires_at timestamptz,
    created_at timestamptz not null default now(),
    started_at timestamptz,
    completed_at timestamptz,
    primary key (import_id, commit_id)
);
create index if not exists import_commits_queue_idx on public.import_commits(status, lease_expires_at, created_at);

create table if not exists public.summarization_attempts (
    id uuid primary key default gen_random_uuid(),
    import_id uuid not null references public.imports(id) on delete cascade,
    commit_id uuid not null references public.commits(id) on delete cascade,
    uploader_id uuid not null references auth.users(id) on delete cascade,
    request_count integer not null default 0,
    reserved_request_count integer not null default 0,
    reserved_input_chars bigint not null default 0,
    unpriced_request_count integer not null default 0,
    cost_usd numeric,
    status text not null,
    error_detail text,
    created_at timestamptz not null default now()
);
alter table public.summarization_attempts
    add column if not exists reserved_request_count integer not null default 0,
    add column if not exists reserved_input_chars bigint not null default 0;
update public.summarization_attempts set reserved_request_count=request_count
where reserved_request_count=0 and request_count>0;
create index if not exists summarization_attempts_user_date_idx on public.summarization_attempts(uploader_id, created_at);
create index if not exists summarization_attempts_date_idx on public.summarization_attempts(created_at);

create table if not exists public.cli_login_requests (
    id uuid primary key default gen_random_uuid(),
    device_code_hash text not null unique,
    browser_code_hash text not null,
    status text not null default 'pending' check (status in ('pending','approved','consumed','expired')),
    user_id uuid references auth.users(id) on delete cascade,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    approved_at timestamptz,
    consumed_at timestamptz
);
create index if not exists cli_login_requests_expiry_idx on public.cli_login_requests(status, expires_at);
create index if not exists cli_login_requests_expires_at_idx on public.cli_login_requests(expires_at);

create table if not exists public.cli_refresh_tokens (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references auth.users(id) on delete cascade,
    token_hash text not null unique,
    expires_at timestamptz not null,
    created_at timestamptz not null default now(),
    last_used_at timestamptz,
    revoked_at timestamptz,
    replaced_by uuid references public.cli_refresh_tokens(id)
);
create index if not exists cli_refresh_tokens_user_idx on public.cli_refresh_tokens(user_id, expires_at);
create index if not exists cli_refresh_tokens_expiry_idx on public.cli_refresh_tokens(expires_at);
create index if not exists cli_refresh_tokens_revoked_idx on public.cli_refresh_tokens(revoked_at) where revoked_at is not null;

alter table public.import_commits enable row level security;
alter table public.summarization_attempts enable row level security;
alter table public.cli_login_requests enable row level security;
alter table public.cli_refresh_tokens enable row level security;

do $$ begin
    alter table public.imports add constraint imports_uploader_id_fkey
        foreign key (uploader_id) references auth.users(id) on delete cascade;
exception when duplicate_object then null;
end $$;
