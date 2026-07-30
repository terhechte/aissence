import { describe, expect, it } from 'vitest';
import { groupTranscript } from './ledger';

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
});
