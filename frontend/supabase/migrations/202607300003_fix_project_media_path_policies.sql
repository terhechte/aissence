drop policy if exists "owners upload project media" on storage.objects;
create policy "owners upload project media" on storage.objects
for insert to authenticated
with check (
  bucket_id = 'project-media'
  and (storage.foldername(storage.objects.name))[1] = auth.uid()::text
  and exists (
    select 1 from public.projects p
    where p.owner_id = auth.uid()
      and p.id::text = (storage.foldername(storage.objects.name))[2]
  )
);

drop policy if exists "project media follows project visibility" on storage.objects;
create policy "project media follows project visibility" on storage.objects
for select
using (
  bucket_id = 'project-media'
  and exists (
    select 1 from public.projects p
    where p.id::text = (storage.foldername(storage.objects.name))[2]
      and (p.visibility = 'published' or p.owner_id = auth.uid())
  )
);
