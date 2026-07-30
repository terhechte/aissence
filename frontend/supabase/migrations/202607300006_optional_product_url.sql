alter table public.projects
  alter column product_url drop not null;

alter table public.projects
  drop constraint if exists projects_product_url_check;

alter table public.projects
  add constraint projects_product_url_check
  check (product_url is null or product_url ~ '^https://');

create or replace function public.create_owned_project(
  p_name text,
  p_slug text,
  p_tagline text,
  p_description text,
  p_product_url text,
  p_repository_label text,
  p_repository_url text
)
returns uuid
language plpgsql
security definer
set search_path = public
as $$
declare
  v_owner_id uuid := auth.uid();
  v_project_id uuid;
begin
  if v_owner_id is null then
    raise exception 'authentication required' using errcode = '42501';
  end if;

  insert into public.projects (
    owner_id,
    slug,
    name,
    tagline,
    description,
    product_url,
    repository_label,
    repository_url
  )
  values (
    v_owner_id,
    p_slug,
    p_name,
    p_tagline,
    p_description,
    nullif(btrim(p_product_url), ''),
    nullif(btrim(p_repository_label), ''),
    nullif(btrim(p_repository_url), '')
  )
  returning id into v_project_id;

  return v_project_id;
end;
$$;

revoke all on function public.create_owned_project(text, text, text, text, text, text, text)
from public;
grant execute on function public.create_owned_project(text, text, text, text, text, text, text)
to authenticated;
