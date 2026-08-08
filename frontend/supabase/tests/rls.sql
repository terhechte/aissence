begin;
create extension if not exists pgtap with schema extensions;
select plan(33);

insert into auth.users (id, email) values
  ('00000000-0000-0000-0000-000000000001', 'creator@example.com'),
  ('00000000-0000-0000-0000-000000000002', 'reader@example.com'),
  ('00000000-0000-0000-0000-000000000003', 'subscriber@example.com');
insert into public.profiles (id, handle, display_name) values
  ('00000000-0000-0000-0000-000000000001', 'creator', 'Creator');
insert into public.creator_plans (id, creator_id, name, description, interval, price_cents, currency, active)
values ('50000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001',
        'Monthly', 'Full prompts', 'monthly', 1000, 'USD', true);
insert into public.projects
  (id, owner_id, slug, name, tagline, description, product_url, repository_label, visibility)
values
  ('10000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001',
   'project', 'Project', 'Tagline', 'Description', 'https://example.com', 'creator/project', 'published');
insert into public.commits
  (id, project_id, commit_hash, public_story, public_beats, prompt_count, reply_count,
   change_message_count, sort_order, public_subline, summary_status, summarizer_version)
values
  ('20000000-0000-0000-0000-000000000001', '10000000-0000-0000-0000-000000000001',
   '8f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2d', 'A privacy-safe public story.',
   '[{"message_index":0,"label":"Request","text":"A safe paraphrase."}]', 1, 1, 1, 0,
   'A compact project overview.', 'succeeded', 2);
insert into public.messages (id, commit_id, kind, sort_order, changed_file_count) values
  ('30000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000001', 'UserWrote', 0, 0),
  ('30000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000001', 'AiWrote', 1, 0);
insert into public.message_sources (message_id, content) values
  ('30000000-0000-0000-0000-000000000001', 'private prompt'),
  ('30000000-0000-0000-0000-000000000002', 'private reply');
insert into public.message_summaries (message_id, summary, summarizer_version) values
  ('30000000-0000-0000-0000-000000000001', 'Subscriber prompt summary', 1);
insert into public.changes (message_id, tool, file_path, diff, sort_order) values
  ('30000000-0000-0000-0000-000000000002', 'Edit', 'src/a.ts', '+private diff', 0);
insert into public.projects
  (id, owner_id, slug, name, tagline, description, product_url, repository_label, visibility)
values
  ('10000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001',
   'publish-gate', 'Publish gate', 'Private draft', 'Atomic publication test',
   'https://example.com', 'creator/publish-gate', 'draft');
insert into public.imports
  (id, project_id, uploader_id, checksum, source_version, status, commit_count,
   submitted_commit_count, completed_at)
values
  ('40000000-0000-0000-0000-000000000001', '10000000-0000-0000-0000-000000000002',
   '00000000-0000-0000-0000-000000000001', 'sha256:publish-gate', 2, 'succeeded', 1, 1, now());
insert into public.commits
  (id, project_id, commit_hash, title, public_story, public_subline, public_beats,
   prompt_count, reply_count, change_message_count, sort_order, summary_status)
values
  ('20000000-0000-0000-0000-000000000002', '10000000-0000-0000-0000-000000000002',
   'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'Pending story', '', '', '[]',
   1, 1, 0, 0, 'skipped');

set local role authenticated;
select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000001', true);
select is((select count(*)::integer from public.projects where id='10000000-0000-0000-0000-000000000001'), 1, 'creator reads project');
select is((select count(*)::integer from public.messages), 2, 'creator reads message metadata');
select is((select count(*)::integer from public.message_sources), 2, 'creator reads all source');
select is((select count(*)::integer from public.message_summaries), 1, 'creator reads paid summaries');
select is((select count(*)::integer from public.changes), 1, 'creator reads changes');
select is((select count(*)::integer from public.project_summary_overviews where project_id='10000000-0000-0000-0000-000000000001'), 1, 'creator reads project overview');
select throws_ok(
  $$select public.publish_ready_project('10000000-0000-0000-0000-000000000002')$$,
  'Generate and review all summaries before publishing (1 remaining).',
  'atomic publish rejects incomplete summaries'
);
select throws_ok(
  $$update public.projects set visibility='published' where id='10000000-0000-0000-0000-000000000002'$$,
  'Publish projects with publish_ready_project().',
  'direct visibility updates cannot bypass summary readiness'
);
reset role;
update public.commits
set public_story='A ready story.', public_subline='A ready subline.',
    summary_status='succeeded', summarizer_version=2
where id='20000000-0000-0000-0000-000000000002';
set local role authenticated;
select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000001', true);
select lives_ok(
  $$select public.publish_ready_project('10000000-0000-0000-0000-000000000002')$$,
  'atomic publish accepts a ready project'
);
select is(
  (select visibility::text from public.projects where id='10000000-0000-0000-0000-000000000002'),
  'published',
  'atomic publish changes project visibility'
);
select lives_ok(
  $$select public.create_owned_project('Draft', 'draft', 'Tagline', 'Description', 'https://draft.test', '', '')$$,
  'creator creates a private draft'
);
select lives_ok(
  $$insert into storage.objects (bucket_id, name, owner_id) values
    ('project-media', '00000000-0000-0000-0000-000000000001/10000000-0000-0000-0000-000000000001/screenshot.png',
     '00000000-0000-0000-0000-000000000001')$$,
  'creator uploads project media'
);
select is((select count(*)::integer from storage.objects where name='00000000-0000-0000-0000-000000000001/10000000-0000-0000-0000-000000000001/screenshot.png'), 1, 'creator reads media');

select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000002', true);
select is((select count(*)::integer from public.commits where id='20000000-0000-0000-0000-000000000001'), 1, 'public reader reads commit stories');
select is((select count(*)::integer from public.project_summary_overviews where project_id='10000000-0000-0000-0000-000000000001'), 1, 'public reader reads published overview');
select is((select count(*)::integer from public.messages), 0, 'public reader cannot enumerate messages');
select is((select count(*)::integer from public.message_sources), 0, 'public reader cannot read prompts');
select is((select count(*)::integer from public.message_summaries), 0, 'public reader cannot read paid summaries');
select is((select count(*)::integer from public.creator_plans), 1, 'public reader sees plan pricing');
select is((select count(*)::integer from public.creator_accounts), 0, 'creator compliance state is private');
select is(public.creator_payments_ready('00000000-0000-0000-0000-000000000001'), false, 'unverified creator cannot accept payments');

reset role;
insert into public.creator_subscriptions (subscriber_id, creator_id, status, source)
values ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 'active', 'manual');
set local role authenticated;
select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000003', true);
select is((select count(*)::integer from public.messages), 2, 'subscriber reads ledger message metadata');
select is((select count(*)::integer from public.message_sources), 1, 'subscriber reads only user source');
select is((select content from public.message_sources limit 1), 'private prompt', 'subscriber receives prompt, not reply');
select is((select count(*)::integer from public.message_summaries), 1, 'subscriber reads prompt summaries');
select is((select summary from public.message_summaries limit 1), 'Subscriber prompt summary', 'subscriber receives paid summary');
select is((select count(*)::integer from public.changes), 0, 'subscriber cannot read private diffs');

reset role;
update public.creator_subscriptions set status='canceled', cancel_at_period_end=true,
  current_period_end=now()+interval '1 day'
where subscriber_id='00000000-0000-0000-0000-000000000003';
set local role authenticated;
select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000003', true);
select is((select count(*)::integer from public.message_sources), 1, 'canceling subscriber keeps period access');

reset role;
update public.creator_subscriptions set current_period_end=now()-interval '1 second'
where subscriber_id='00000000-0000-0000-0000-000000000003';
set local role authenticated;
select is((select count(*)::integer from public.messages), 0, 'expired subscriber loses message metadata');
select is((select count(*)::integer from public.message_sources), 0, 'expired subscriber loses source');

select set_config('request.jwt.claim.sub', '00000000-0000-0000-0000-000000000002', true);
select throws_ok(
  $$insert into public.projects (owner_id, slug, name, tagline, description, product_url, repository_label)
    values ('00000000-0000-0000-0000-000000000001', 'stolen', 'Stolen', 'x', 'x', 'https://x.test', 'x/y')$$,
  'new row violates row-level security policy for table "projects"',
  'reader cannot create projects for creator'
);
select throws_ok(
  $$insert into storage.objects (bucket_id, name, owner_id) values
    ('project-media', '00000000-0000-0000-0000-000000000002/10000000-0000-0000-0000-000000000001/stolen.png',
     '00000000-0000-0000-0000-000000000002')$$,
  'new row violates row-level security policy for table "objects"',
  'reader cannot upload media for another creator'
);

reset role;
set local role anon;
select is((select count(*)::integer from public.project_summary_overviews where project_id='10000000-0000-0000-0000-000000000001'), 1, 'anonymous visitor reads published overview');

select * from finish();
rollback;
