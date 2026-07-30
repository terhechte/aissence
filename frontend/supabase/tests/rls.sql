begin;
create extension if not exists pgtap with schema extensions;
select plan(15);

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
insert into public.messages (id, commit_id, kind, summary, preview, sort_order) values
  ('30000000-0000-0000-0000-000000000001', '20000000-0000-0000-0000-000000000001', 'UserWrote', 'User summary', 'private prompt', 0),
  ('30000000-0000-0000-0000-000000000002', '20000000-0000-0000-0000-000000000001', 'AiWrote', null, 'private reply', 1);
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
insert into public.imports (
  id, project_id, uploader_id, checksum, source_version, status
)
select
  '40000000-0000-0000-0000-000000000001',
  id,
  '00000000-0000-0000-0000-000000000001',
  'priced-snapshot',
  1,
  'processing'
from public.projects
where slug = 'draft';
select lives_ok(
  $$select public.replace_project_snapshot(
    (select id from public.projects where slug = 'draft'),
    '40000000-0000-0000-0000-000000000001',
    'priced-snapshot',
    1,
    '{
      "commits": [{
        "hash": "9f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2e",
        "title": "Priced import",
        "date": "2026-07-30T10:00:00Z",
        "stats": {"files_changed": 2, "lines_added": 3, "lines_deleted": 1},
        "summary": {
          "user_requested": "Price this.",
          "ai_answered": "Priced it.",
          "ai_implemented": "Stored pricing.",
          "meta": "",
          "tokens": {"input": 1000, "output": 100},
          "agents": [{"provider": "Codex", "model": "gpt-test"}],
          "messages": [{
            "kind": "AiChanged",
            "changes": [
              {"tool": "Edit", "file": "src/a.ts", "diff": "+a"},
              {"tool": "Edit", "file": "src/b.ts", "diff": "+b"},
              {"tool": "Edit", "file": "src/a.ts", "diff": "+again"}
            ],
            "timestamp": "2026-07-30T10:01:00Z"
          }]
        },
        "pricing": {
          "input_cost_usd": 0.01,
          "output_cost_usd": 0.0023,
          "potential_cost_usd": 0.0123,
          "model_ids": ["openai/gpt-test"],
          "priced_at": "2026-07-30T10:02:00Z"
        }
      }]
    }'::jsonb
  )$$,
  'snapshot import stores pricing and activity metadata'
);
select is(
  (select potential_cost_usd from public.commits where commit_hash = '9f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2e'),
  0.01230000::numeric,
  'commit potential cost is persisted'
);
select is(
  (
    select changed_file_count
    from public.messages m
    join public.commits c on c.id = m.commit_id
    where c.commit_hash = '9f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2e'
  ),
  2,
  'message stores the number of distinct changed files'
);
select lives_ok(
  $$insert into storage.objects (bucket_id, name, owner_id)
    values (
      'project-media',
      '00000000-0000-0000-0000-000000000001/10000000-0000-0000-0000-000000000001/screenshot.png',
      '00000000-0000-0000-0000-000000000001'
    )$$,
  'creator can upload media for their project'
);
select is(
  (select count(*)::integer from storage.objects where bucket_id = 'project-media'),
  1,
  'creator can read their project media'
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
select throws_ok(
  $$insert into storage.objects (bucket_id, name, owner_id)
    values (
      'project-media',
      '00000000-0000-0000-0000-000000000002/10000000-0000-0000-0000-000000000001/stolen.png',
      '00000000-0000-0000-0000-000000000002'
    )$$,
  'new row violates row-level security policy for table "objects"',
  'reader cannot upload media for another creator project'
);
select is((select count(*)::integer from public.changes), 0, 'reader cannot see creator changes');

select * from finish();
rollback;
