-- Product decisions that can be enforced before a payment/KYC provider is chosen.

do $$
begin
  create type public.creator_plan_interval as enum ('monthly', 'annual');
exception when duplicate_object then null;
end;
$$;

do $$
begin
  create type public.project_comment_visibility as enum ('public', 'subscribers');
exception when duplicate_object then null;
end;
$$;

do $$
begin
  create type public.creator_account_status as enum (
    'onboarding',
    'active',
    'paused',
    'suspended',
    'payout_restricted'
  );
exception when duplicate_object then null;
end;
$$;

do $$
begin
  create type public.creator_kyc_status as enum ('not_started', 'pending', 'verified', 'rejected');
exception when duplicate_object then null;
end;
$$;

do $$
begin
  create type public.creator_payout_status as enum ('not_ready', 'pending', 'enabled', 'restricted');
exception when duplicate_object then null;
end;
$$;

alter type public.subscription_source add value if not exists 'provider';

alter table public.projects
  add column if not exists comment_visibility public.project_comment_visibility not null default 'subscribers';

create table if not exists public.creator_accounts (
  creator_id uuid primary key references public.profiles(id) on delete cascade,
  status public.creator_account_status not null default 'onboarding',
  kyc_status public.creator_kyc_status not null default 'not_started',
  payout_status public.creator_payout_status not null default 'not_ready',
  provider text,
  provider_account_id text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.creator_plans (
  id uuid primary key default gen_random_uuid(),
  creator_id uuid not null references public.profiles(id) on delete cascade,
  name text not null check (char_length(name) between 1 and 80),
  description text check (description is null or char_length(description) <= 500),
  interval public.creator_plan_interval not null,
  price_cents integer not null check (price_cents > 0 and price_cents <= 100000000),
  currency text not null default 'USD' check (currency ~ '^[A-Z]{3}$'),
  active boolean not null default false,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (creator_id, interval)
);

insert into public.creator_accounts (creator_id)
select id from public.profiles
on conflict (creator_id) do nothing;

create or replace function public.create_creator_account()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  insert into public.creator_accounts (creator_id)
  values (new.id)
  on conflict (creator_id) do nothing;
  return new;
end;
$$;

drop trigger if exists profiles_create_creator_account on public.profiles;
create trigger profiles_create_creator_account after insert on public.profiles
for each row execute function public.create_creator_account();

alter table public.creator_subscriptions
  add column if not exists plan_id uuid references public.creator_plans(id) on delete set null,
  add column if not exists cancel_at_period_end boolean not null default false,
  add column if not exists canceled_at timestamptz;

create index if not exists creator_plans_public_idx
  on public.creator_plans (creator_id, active, interval);
create index if not exists creator_subscriptions_period_idx
  on public.creator_subscriptions (subscriber_id, status, current_period_end);

drop trigger if exists creator_accounts_touch_updated_at on public.creator_accounts;
create trigger creator_accounts_touch_updated_at before update on public.creator_accounts
for each row execute function public.touch_updated_at();

drop trigger if exists creator_plans_touch_updated_at on public.creator_plans;
create trigger creator_plans_touch_updated_at before update on public.creator_plans
for each row execute function public.touch_updated_at();

alter table public.creator_accounts enable row level security;
alter table public.creator_plans enable row level security;

drop policy if exists "creators can read their private account state" on public.creator_accounts;
create policy "creators can read their private account state" on public.creator_accounts
for select to authenticated
using (creator_id = auth.uid());

drop policy if exists "active plans are public" on public.creator_plans;
create policy "active plans are public" on public.creator_plans
for select
using (active = true or creator_id = auth.uid());

drop policy if exists "creators create plans" on public.creator_plans;
create policy "creators create plans" on public.creator_plans
for insert to authenticated
with check (creator_id = auth.uid());

drop policy if exists "creators update plans" on public.creator_plans;
create policy "creators update plans" on public.creator_plans
for update to authenticated
using (creator_id = auth.uid())
with check (creator_id = auth.uid());

drop policy if exists "creators delete plans" on public.creator_plans;
create policy "creators delete plans" on public.creator_plans
for delete to authenticated
using (creator_id = auth.uid());

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
      and (
        (
          s.status in ('active', 'trialing')
          and (s.current_period_end is null or s.current_period_end > now())
        )
        or (
          s.status = 'canceled'
          and s.cancel_at_period_end = true
          and s.current_period_end > now()
        )
      )
  );
$$;

create or replace function public.creator_payments_ready(p_creator_id uuid)
returns boolean
language sql stable security definer
set search_path = public
as $$
  select exists (
    select 1
    from public.creator_accounts a
    where a.creator_id = p_creator_id
      and a.status = 'active'
      and a.kyc_status = 'verified'
      and a.payout_status = 'enabled'
  );
$$;

revoke all on function public.creator_payments_ready(uuid) from public;
grant execute on function public.creator_payments_ready(uuid) to anon, authenticated;

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
    subscriber_id,
    creator_id,
    status,
    source,
    current_period_end,
    cancel_at_period_end,
    canceled_at,
    plan_id
  ) values (
    p_subscriber_id,
    p_creator_id,
    'active',
    'manual',
    p_expires_at,
    false,
    null,
    null
  )
  on conflict (subscriber_id, creator_id) do update
    set status = 'active',
        source = 'manual',
        current_period_end = excluded.current_period_end,
        cancel_at_period_end = false,
        canceled_at = null,
        plan_id = null,
        updated_at = now()
  returning id into v_id;
  return v_id;
end;
$$;

revoke all on function public.grant_manual_subscription(uuid, uuid, timestamptz) from public;
grant execute on function public.grant_manual_subscription(uuid, uuid, timestamptz) to service_role;
