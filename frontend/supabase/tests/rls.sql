begin;
create extension if not exists pgtap with schema extensions;
select plan(9);

insert into auth.users (id, email)
values
  ('00000000-0000-0000-0000-000000000001', 'creator@example.com'),
  ('00000000-0000-0000-0000-000000000002', 'reader@example.com'),
  ('00000000-0000-0000-0000-000000000003', 'subscriber@example.com');

insert into public.profiles (id, handle, display_name) values
  ('00000000-0000-0000-0000-000000000001', 'creator', 'Creator');
insert into public.projects (
  id, owner_id, slug, name, tagline, description, product_url, repository_label, visibility
) values (
  '10000000-0000-0000-0000-000000000001',
  '00000000-0000-0000-0000-000000000001',
  'project', 'Project', 'Tagline', 'Description', 'https://example.com', 'creator/project', 'published'
);
insert into public.commits (
  id, project_id, commit_hash, user_requested, ai_answered, ai_implemented, sort_order
) values (
  '20000000-0000-0000-0000-000000000001',
  '10000000-0000-0000-0000-000000000001',
  '8f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2d',
  'Asked', 'Answered', 'Built', 0
);
insert into public.messages (id, commit_id, kind, summary, sort_order) values
  ('30000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000001', 'UserWrote', 'User summary', 0),
  ('30000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000001', 'AiWrote', 'AI summary', 1);
insert into public.message_sources (message_id, content) values
  ('30000000-0000-0000-0000-000000000001', 'private prompt'),
  ('30000000-0000-0000-0000-000000000002', 'private reply');

set local role authenticated;
select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000001', true);
select is((select count(*)::integer from public.projects), 1, 'creator can read published project');
select is((select count(*)::integer from public.message_sources), 2, 'creator can read all source');
select lives_ok(
  $$select public.create_owned_project(
    'Draft', 'draft', 'Tagline', 'Description', 'https://draft.test', '', ''
  )$$,
  'creator can create a private draft through the owner-bound function'
);

select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000002', true);
select is((select count(*)::integer from public.commits), 1, 'unrelated reader can read summaries');
select is((select count(*)::integer from public.message_sources), 0, 'unrelated reader cannot read source');

reset role;
insert into public.creator_subscriptions (subscriber_id, creator_id, status, source)
values (
  '00000000-0000-0000-0000-000000000003',
  '00000000-0000-0000-0000-000000000001',
  'active', 'manual'
);
set local role authenticated;
select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000003', true);
select is((select count(*)::integer from public.message_sources), 1, 'subscriber reads only user source');
select is(
  (select content from public.message_sources limit 1),
  'private prompt',
  'subscriber receives prompt rather than reply'
);

select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000002', true);
select throws_ok(
  $$insert into public.projects (owner_id, slug, name, tagline, description, product_url, repository_label)
    values ('00000000-0000-0000-0000-000000000001', 'stolen', 'Stolen', 'x', 'x', 'https://x.test', 'x/y')$$,
  'new row violates row-level security policy for table "projects"',
  'reader cannot create projects for creator'
);
select is((select count(*)::integer from public.changes), 0, 'reader cannot see creator changes');

select * from finish();
rollback;
