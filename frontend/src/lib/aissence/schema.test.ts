import { describe, expect, it } from 'vitest';
import { projectMetadataSchema, repositoryFields, screenshotMetadataSchema } from './project';
import { AissenceValidationError, parseAissence } from './schema';

const hash = '8f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2d';

function userMessage(content = 'Please add retries.') {
  return { kind: 'UserWrote', content, changes: [], timestamp: '2026-06-20T09:14:02Z' };
}

function validFile(overrides: Record<string, unknown> = {}) {
  const sourceMessage = userMessage();
  return {
    version: 2,
    project: {
      source_id: 'd9428888-122b-11e1-b85c-61cd3cbb3210',
      name: 'Emyn',
      repository: 'github.com/terhechte/Emyn'
    },
    commits: [{
      commit_hash: hash,
      source_digest: `sha256:${'0'.repeat(64)}`,
      sanitizer_version: 1,
      title: 'Add retry support',
      date: '2026-06-20T09:18:03Z',
      stats: { files_changed: 3, lines_added: 21, lines_deleted: 4 },
      files: [],
      source: {
        messages: [sourceMessage],
        agents: [{ provider: 'Codex', model: 'gpt-test', tokens: { input: 100, output: 20 } }],
        tokens: { input: 100, output: 20 }
      },
      summary: {
        public: {
          summary: 'Retry support was requested and implemented with backoff.',
          beats: [{ message_index: 0, label: 'Initial request', text: 'The creator asked for resilient retries.' }]
        },
        messages: [{ ...sourceMessage, subscriber_summary: 'Add retry support.' }],
        agents: [{ provider: 'Codex', model: 'gpt-test', tokens: { input: 100, output: 20 } }],
        tokens: { input: 100, output: 20 }
      }
    }],
    ignored: [],
    ...overrides
  };
}

describe('parseAissence', () => {
  it('parses the v2 source/summary contract', () => {
    const result = parseAissence(JSON.stringify(validFile()));
    expect(result.file.version).toBe(2);
    expect(result.snapshot.commits[0]!.commit_hash).toBe(hash);
    expect(result.snapshot.commits[0]!.source.messages[0]!.content).toBe('Please add retries.');
    expect(result.snapshot.commits[0]!.summary?.public.beats).toHaveLength(1);
  });

  it('accepts a complete unsummarized upload', () => {
    const input = validFile();
    delete (input.commits[0] as any).summary;
    expect(parseAissence(JSON.stringify(input)).snapshot.commits).toHaveLength(1);
  });

  it('keeps subscriber summaries separate from source', () => {
    const input = validFile();
    (input.commits[0] as any).source.messages[0].subscriber_summary = 'leak';
    expect(() => parseAissence(JSON.stringify(input))).toThrow(AissenceValidationError);
  });

  it('rejects public beats that do not point to a prompt', () => {
    const input = validFile();
    (input.commits[0] as any).summary.public.beats[0].message_index = 3;
    expect(() => parseAissence(JSON.stringify(input))).toThrow(/public beat/i);
  });

  it('ignores commits listed in ignored', () => {
    expect(parseAissence(JSON.stringify(validFile({ ignored: [hash] }))).snapshot.commits).toEqual([]);
  });

  it.each([
    ['old version', { ...validFile(), version: 1 }],
    ['bad digest', (() => { const value = validFile(); value.commits[0].source_digest = 'bad'; return value; })()],
    ['negative stats', (() => { const value = validFile(); value.commits[0].stats.lines_added = -1; return value; })()]
  ])('rejects %s', (_name, input) => {
    expect(() => parseAissence(JSON.stringify(input))).toThrow(AissenceValidationError);
  });

  it('preserves hostile prompt strings as inert data', () => {
    const input = validFile();
    (input.commits[0] as any).source.messages[0].content = '<script>globalThis.compromised=true</script>';
    const parsed = parseAissence(JSON.stringify(input));
    expect(parsed.snapshot.commits[0]!.source.messages[0]!.content).toContain('<script>');
    expect((globalThis as any).compromised).toBeUndefined();
  });

  it('reports malformed JSON cleanly', () => {
    expect(() => parseAissence('{nope')).toThrow('not valid JSON');
  });
});

describe('optional project metadata', () => {
  it('derives repository fields from a GitHub reference', () => {
    expect(repositoryFields('github.com/terhechte/Emyn')).toEqual({
      label: 'terhechte/Emyn',
      url: 'https://github.com/terhechte/Emyn'
    });
  });

  it('accepts optional project URLs and screenshot alt text', () => {
    const project = projectMetadataSchema.parse({
      name: 'Project', slug: 'project', tagline: 'A project', description: 'Description',
      product_url: '', repository_label: '', repository_url: ''
    });
    expect(project.product_url).toBeNull();
    expect(project.repository_label).toBeNull();
    expect(screenshotMetadataSchema.parse({
      storage_path: 'creator/project/image.png', alt_text: '', caption: '', sort_order: 0
    }).alt_text).toBe('');
  });
});
