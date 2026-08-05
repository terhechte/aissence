alter table public.commits
    add column if not exists public_prompt_summaries boolean not null default false;

-- A complete set of locally generated prompt summaries may be read publicly.
-- Raw prompt sources and server-generated subscriber summaries remain gated.
drop policy if exists "messages are owner or entitled" on public.messages;
drop policy if exists "messages are owner entitled or locally public" on public.messages;
create policy "messages are owner entitled or locally public" on public.messages for select
using (exists (
    select 1
    from public.commits c
    join public.projects p on p.id = c.project_id
    where c.id = commit_id
      and (
        p.owner_id = auth.uid()
        or (p.visibility = 'published' and public.has_creator_access(p.owner_id))
        or (p.visibility = 'published' and c.public_prompt_summaries)
      )
));

drop policy if exists "subscriber summaries are owner or entitled" on public.message_summaries;
drop policy if exists "prompt summaries are owner entitled or locally public" on public.message_summaries;
create policy "prompt summaries are owner entitled or locally public" on public.message_summaries for select
using (exists (
    select 1
    from public.messages m
    join public.commits c on c.id = m.commit_id
    join public.projects p on p.id = c.project_id
    where m.id = message_id
      and (
        p.owner_id = auth.uid()
        or (p.visibility = 'published' and public.has_creator_access(p.owner_id))
        or (p.visibility = 'published' and c.public_prompt_summaries)
      )
));
