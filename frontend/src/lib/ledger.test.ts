import { describe, expect, it } from 'vitest';
import { abridgeTranscript, groupTranscript, redactPublicMessage } from './ledger';

describe('redactPublicMessage', () => {
  it('does not serialize retained prompt previews or private source relations', () => {
    expect(
      redactPublicMessage({
        id: 'message',
        kind: 'UserWrote',
        summary: 'Public summary',
        preview: 'Private prompt preview',
        message_sources: [{ content: 'Private prompt' }],
        changes: [{ diff: 'Private diff' }]
      })
    ).toEqual({ id: 'message', kind: 'UserWrote', summary: 'Public summary' });
  });
});

describe('groupTranscript', () => {
  it('groups agent activity under the preceding prompt', () => {
    const turns = groupTranscript([
      { id: 'u1', kind: 'UserWrote' },
      { id: 'a1', kind: 'AiWrote' },
      { id: 'c1', kind: 'AiChanged', changed_file_count: 2 },
      { id: 'a2', kind: 'AiWrote' },
      { id: 'u2', kind: 'UserWrote' },
      {
        id: 'c2',
        kind: 'AiChanged',
        changes: [{ file_path: 'a.ts' }, { file_path: 'a.ts' }, { file_path: 'b.ts' }]
      }
    ]);

    expect(turns).toHaveLength(2);
    expect(turns[0]).toMatchObject({ replyCount: 2, changedFileCount: 2 });
    expect(turns[0].activity.map((message) => message.id)).toEqual(['a1', 'c1', 'a2']);
    expect(turns[1]).toMatchObject({ replyCount: 0, changedFileCount: 2 });
  });

  it('keeps server-provided public activity counts without detailed rows', () => {
    const [turn] = groupTranscript([
      {
        id: 'u1',
        kind: 'UserWrote',
        following_reply_count: 3,
        following_changed_file_count: 5
      }
    ]);
    expect(turn).toMatchObject({ replyCount: 3, changedFileCount: 5, activity: [] });
  });

  it('keeps the first prompt, hides redundant prompts, and samples summarized prompts', () => {
    const turns = groupTranscript([
      { id: 'u1', kind: 'UserWrote', summary: 'Start the project', source: 'Start the project' },
      { id: 'u2', kind: 'UserWrote', summary: 'please continue', source: 'please continue' },
      { id: 'u3', kind: 'UserWrote', summary: 'Add the settings screen', source: 'Please add the settings screen.' },
      { id: 'u4', kind: 'UserWrote', summary: 'please continue', source: 'please continue' },
      { id: 'u5', kind: 'UserWrote', summary: 'Add keyboard navigation', source: 'Please add keyboard navigation.' },
      { id: 'u6', kind: 'UserWrote', summary: 'please continue', source: 'please continue' },
      { id: 'u7', kind: 'UserWrote', summary: 'Improve the empty state', source: 'Please improve the empty state.' },
      { id: 'u8', kind: 'UserWrote', summary: 'please continue', source: 'please continue' },
      { id: 'u9', kind: 'UserWrote', summary: 'Add import validation', source: 'Please add import validation.' },
      { id: 'u10', kind: 'UserWrote', summary: 'please continue', source: 'please continue' },
      { id: 'u11', kind: 'UserWrote', summary: 'Polish the ledger', source: 'Please polish the ledger.' },
      { id: 'u12', kind: 'UserWrote', summary: 'please continue', source: 'please continue' },
      { id: 'u13', kind: 'UserWrote', summary: 'Add a dark theme', source: 'Please add a dark theme.' }
    ]);

    const items = abridgeTranscript(turns, (message) => message.source, 'commit-1');
    const visibleIds = items.filter((item) => item.kind === 'turn').map((item) => item.turn.asked.id);
    const hidden = items.filter((item) => item.kind === 'hidden');

    expect(visibleIds).toContain('u1');
    expect(visibleIds).toHaveLength(3);
    expect(hidden.reduce((total, item) => total + (item.kind === 'hidden' ? item.promptCount : 0), 0)).toBe(10);
  });

  it('uses the smaller sample sizes for shorter transcripts', () => {
    const makeTurns = (count: number) =>
      groupTranscript(
        Array.from({ length: count }, (_, index) => ({
          id: `u${index}`,
          kind: 'UserWrote',
          summary: `Summary ${index}`,
          source: `Full prompt ${index}`
        }))
      );
    const content = (message: any) => message.source;

    expect(abridgeTranscript(makeTurns(6), content, 'six').filter((item) => item.kind === 'turn')).toHaveLength(2);
    expect(abridgeTranscript(makeTurns(11), content, 'eleven').filter((item) => item.kind === 'turn')).toHaveLength(3);
  });
});
