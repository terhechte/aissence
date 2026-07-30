alter table public.projects
  alter column repository_label drop not null;

alter table public.projects
  drop constraint if exists projects_repository_label_check;

alter table public.projects
  add constraint projects_repository_label_check
  check (repository_label is null or char_length(repository_label) between 1 and 160);

alter table public.project_screenshots
  alter column alt_text set default '';

alter table public.project_screenshots
  drop constraint if exists project_screenshots_alt_text_check;

alter table public.project_screenshots
  add constraint project_screenshots_alt_text_check
  check (char_length(alt_text) between 0 and 300);

insert into storage.buckets (id, name, public, file_size_limit, allowed_mime_types)
values (
  'profile-images',
  'profile-images',
  true,
  5242880,
  array['image/jpeg', 'image/png', 'image/webp']
)
on conflict (id) do update
set public = excluded.public,
    file_size_limit = excluded.file_size_limit,
    allowed_mime_types = excluded.allowed_mime_types;

create policy "users upload their profile images" on storage.objects
for insert to authenticated
with check (
  bucket_id = 'profile-images'
  and (storage.foldername(name))[1] = auth.uid()::text
);

create policy "users update their profile images" on storage.objects
for update to authenticated
using (
  bucket_id = 'profile-images'
  and (storage.foldername(name))[1] = auth.uid()::text
)
with check (
  bucket_id = 'profile-images'
  and (storage.foldername(name))[1] = auth.uid()::text
);

create policy "users delete their profile images" on storage.objects
for delete to authenticated
using (
  bucket_id = 'profile-images'
  and (storage.foldername(name))[1] = auth.uid()::text
);

drop policy if exists "creators create projects" on public.projects;
create policy "creators create projects" on public.projects
for insert to authenticated
with check (auth.uid() is not null and owner_id = auth.uid());

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
    p_product_url,
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
