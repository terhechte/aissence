alter table public.projects
  alter column tagline set default '',
  alter column description set default '';

alter table public.projects
  drop constraint if exists projects_tagline_check,
  drop constraint if exists projects_description_check;

alter table public.projects
  add constraint projects_tagline_check
    check (char_length(tagline) <= 240),
  add constraint projects_description_check
    check (char_length(description) <= 3000);
