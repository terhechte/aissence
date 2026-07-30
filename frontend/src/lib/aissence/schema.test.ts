import { describe, expect, it } from 'vitest';
import { projectMetadataSchema, screenshotMetadataSchema } from './project';
import { AissenceValidationError, parseAissence } from './schema';

const hash = '8f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2d';

function validFile(overrides: Record<string, unknown> = {}) {
  return {
    version: 1,
    commits: {
      [hash]: {
        signature: 17712209936023450000,
        title: 'Add retry support',
        date: '2026-06-20T09:18:03Z',
        stats: { files_changed: 3, lines_added: 21, lines_deleted: 4 },
        summary: {
          user_requested: 'Add retries.',
          ai_answered: 'Use backoff.',
          ai_implemented: 'Added retries.',
          messages: [
            {
              kind: 'UserWrote',
              summary: 'Asked for retries.',
              content: 'Please add retries.',
              changes: [],
              timestamp: '2026-06-20T09:14:02Z'
            }
          ],
          agents: [
            {
              provider: 'Codex',
              model: 'gpt-test',
              tokens: { input: 100, output: 20, cache_read: 80 }
            }
          ],
          meta: 'Retry support.',
          tokens: { input: 100, output: 20 }
        }
      }
    },
    ignored: [],
    ...overrides
  };
}

describe('parseAissence', () => {
  it('normalizes a valid v1 file', () => {
    const result = parseAissence(JSON.stringify(validFile()));
    expect(result.file.version).toBe(1);
    expect(result.snapshot.commits).toHaveLength(1);
    expect(result.snapshot.commits[0].hash).toBe(hash);
    expect(result.snapshot.commits[0].summary.messages[0].content).toBe('Please add retries.');
    expect(result.snapshot.commits[0].summary.agents[0].tokens).toEqual({
      input: 100,
      output: 20,
      cache_read: 80
    });
  });

  it('retains a summary for every user prompt without a per-commit cap', () => {
    const input = validFile();
    (input.commits[hash] as any).summary.messages = Array.from({ length: 20 }, (_, index) => ({
      kind: 'UserWrote',
      summary: `Prompt summary ${index + 1}`,
      content: `Complete prompt ${index + 1}`,
      changes: [],
      timestamp: `2026-06-20T09:${String(index).padStart(2, '0')}:00Z`
    }));

    const messages = parseAissence(JSON.stringify(input)).snapshot.commits[0].summary.messages;
    expect(messages).toHaveLength(20);
    expect(messages.map((message) => message.summary)).toEqual(
      Array.from({ length: 20 }, (_, index) => `Prompt summary ${index + 1}`)
    );
  });

  it('accepts legacy v1 entries and emits warnings', () => {
    const input = validFile();
    delete (input.commits[hash] as any).title;
    delete (input.commits[hash] as any).date;
    delete (input.commits[hash] as any).stats;
    const result = parseAissence(JSON.stringify(input));
    expect(result.warnings).toHaveLength(3);
  });

  it('ignores unknown fields and ignored commits', () => {
    const input = validFile({ ignored: [hash], future_field: 'ok' });
    (input.commits[hash] as any).future_field = { anything: true };
    expect(parseAissence(JSON.stringify(input)).snapshot.commits).toEqual([]);
  });

  it('does not depend on exact signature precision', () => {
    const raw = JSON.stringify(validFile()).replace(
      '"signature":17712209936023450000',
      '"signature":18446744073709551615'
    );
    expect(parseAissence(raw).snapshot.commits).toHaveLength(1);
  });

  it.each([
    ['unknown version', { ...validFile(), version: 2 }],
    ['bad hash', { ...validFile(), commits: { bad: validFile().commits[hash] } }],
    [
      'negative stats',
      {
        ...validFile(),
        commits: {
          [hash]: { ...validFile().commits[hash], stats: { files_changed: 1, lines_added: -1, lines_deleted: 0 } }
        }
      }
    ],
    [
      'invalid date',
      {
        ...validFile(),
        commits: { [hash]: { ...validFile().commits[hash], date: 'last Thursday' } }
      }
    ],
    [
      'negative per-agent tokens',
      (() => {
        const input = validFile();
        (input.commits[hash] as any).summary.agents[0].tokens.input = -1;
        return input;
      })()
    ]
  ])('rejects %s', (_name, input) => {
    expect(() => parseAissence(JSON.stringify(input))).toThrow(AissenceValidationError);
  });

  it('rejects changes on non-change messages', () => {
    const input = validFile();
    (input.commits[hash] as any).summary.messages[0].changes = [
      { tool: 'Edit', file: 'src/a.ts', diff: '+unsafe' }
    ];
    expect(() => parseAissence(JSON.stringify(input))).toThrow(/Only AiChanged/);
  });

  it('preserves hostile strings as plain data', () => {
    const input = validFile();
    (input.commits[hash] as any).summary.messages[0].content =
      '<script>globalThis.compromised=true</script>';
    const parsed = parseAissence(JSON.stringify(input));
    expect(parsed.snapshot.commits[0].summary.messages[0].content).toContain('<script>');
    expect((globalThis as any).compromised).toBeUndefined();
  });

  it('reports malformed JSON cleanly', () => {
    expect(() => parseAissence('{nope')).toThrow('not valid JSON');
  });
});

describe('optional project metadata', () => {
  it('accepts a project without a repository label', () => {
    const parsed = projectMetadataSchema.parse({
      name: 'Project',
      slug: 'project',
      tagline: 'A project',
      description: 'Description',
      product_url: 'https://example.com',
      repository_label: '',
      repository_url: ''
    });
    expect(parsed.repository_label).toBeNull();
  });

  it('accepts a project without a product URL', () => {
    const parsed = projectMetadataSchema.parse({
      name: 'Project',
      slug: 'project',
      tagline: 'A project',
      description: 'Description',
      product_url: '',
      repository_label: '',
      repository_url: ''
    });
    expect(parsed.product_url).toBeNull();
  });

  it('accepts a screenshot without alt text', () => {
    const parsed = screenshotMetadataSchema.parse({
      storage_path: 'creator/project/image.png',
      alt_text: '',
      caption: '',
      sort_order: 0
    });
    expect(parsed.alt_text).toBe('');
  });
});
