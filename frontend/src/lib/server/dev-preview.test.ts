import { describe, expect, it } from 'vitest';
import { previewDataFromDocument } from './dev-preview';

const document = {
  version: 2,
  project: {
    source_id: '86841816-16de-4c39-9e72-7fd68b06fe96',
    name: 'Preview project',
    repository: 'git@github.com:example/preview-project.git'
  },
  commits: [
    {
      commit_hash: 'a'.repeat(40),
      source_digest: `sha256:${'b'.repeat(64)}`,
      sanitizer_version: 2,
      title: 'Add preview workflow',
      date: '2026-08-09T12:00:00Z',
      stats: { files_changed: 2, lines_added: 40, lines_deleted: 3 },
      files: [],
      source: {
        messages: [
          {
            kind: 'UserWrote',
            content: 'Please add a preview.',
            changes: [],
            timestamp: '2026-08-09T11:55:00Z'
          },
          {
            kind: 'AiChanged',
            changes: [{ tool: 'edit', file: 'scripts/preview.sh', diff: '+preview' }],
            timestamp: '2026-08-09T11:56:00Z'
          }
        ],
        agents: [{ provider: 'Codex', model: 'gpt-5' }],
        tokens: { input: 100, output: 25 }
      },
      summary: {
        summarizer_version: 2,
        public: {
          subline: 'A one-command local preview loop.',
          summary: 'The change adds a quick local preview workflow.',
          beats: [{ message_index: 0, label: 'Goal', text: 'Shorten the feedback loop.' }],
          origin: 'model'
        },
        messages: [
          {
            kind: 'UserWrote',
            content: 'Please add a preview.',
            changes: [],
            timestamp: '2026-08-09T11:55:00Z',
            subscriber_summary: 'Add a fast local preview.'
          },
          {
            kind: 'AiChanged',
            changes: [{ tool: 'edit', file: 'scripts/preview.sh', diff: '+preview' }],
            timestamp: '2026-08-09T11:56:00Z'
          }
        ],
        agents: [],
        tokens: { input: 100, output: 25 },
        prompt_failures: []
      }
    }
  ],
  ignored: []
};

describe('previewDataFromDocument', () => {
  it('maps a validated .aissence document into the ledger shape', () => {
    const preview = previewDataFromDocument(JSON.stringify(document));

    expect(preview.project).toMatchObject({
      name: 'Preview project',
      repository_label: 'example/preview-project',
      repository_url: 'https://github.com/example/preview-project'
    });
    expect(preview.commits[0]).toMatchObject({
      public_story: 'The change adds a quick local preview workflow.',
      public_prompt_summaries: true,
      prompt_count: 1,
      reply_count: 0,
      change_message_count: 1,
      token_input: 100,
      token_output: 25
    });
    expect(preview.commits[0].messages[0].subscriber_summary).toBe('Add a fast local preview.');
    expect(preview.commits[0].messages[1].changes[0].file_path).toBe('scripts/preview.sh');
  });
});
