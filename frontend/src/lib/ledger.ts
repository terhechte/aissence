export interface TranscriptTurn {
  asked: any | null;
  activity: any[];
  replyCount: number;
  changedFileCount: number;
}

/** Remove retained prompt previews and source relations before serializing a free view. */
export function redactPublicMessage(message: any) {
  const { preview: _preview, message_sources: _sources, content: _content, changes: _changes, ...safe } = message;
  return safe;
}

export type PublicTranscriptItem =
  | { kind: 'turn'; turn: TranscriptTurn }
  | {
      kind: 'hidden';
      promptCount: number;
      replyCount: number;
      changedFileCount: number;
    };

function messageChangedFileCount(message: any): number {
  if (message.changed_file_count !== undefined && message.changed_file_count !== null) {
    const persisted = Number(message.changed_file_count);
    if (persisted > 0 || !(message.changes ?? []).length) return persisted;
  }
  return new Set((message.changes ?? []).map((change: any) => change.file_path)).size;
}

export function groupTranscript(messages: any[]): TranscriptTurn[] {
  const turns: TranscriptTurn[] = [];
  let current: TranscriptTurn | null = null;

  for (const message of messages) {
    if (message.kind === 'UserWrote') {
      current = {
        asked: message,
        activity: [],
        replyCount: Number(message.following_reply_count ?? 0),
        changedFileCount: Number(message.following_changed_file_count ?? 0)
      };
      turns.push(current);
      continue;
    }

    if (!current) {
      current = { asked: null, activity: [], replyCount: 0, changedFileCount: 0 };
      turns.push(current);
    }
    current.activity.push(message);
    if (message.kind === 'AiWrote') current.replyCount += 1;
    if (message.kind === 'AiChanged') {
      current.changedFileCount += messageChangedFileCount(message);
    }
  }

  return turns;
}

function normalizedText(value: unknown): string {
  return String(value ?? '').trim().replace(/\s+/g, ' ');
}

function selectionScore(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

export function abridgeTranscript(
  turns: TranscriptTurn[],
  promptContent: (message: any) => string | null,
  seed: string
): PublicTranscriptItem[] {
  const firstPromptIndex = turns.findIndex((turn) => turn.asked);
  if (firstPromptIndex === -1) return [];

  const remainingTurns = turns.slice(firstPromptIndex + 1).filter((turn) => turn.asked);
  const summarizedTurns = remainingTurns.filter((turn) => {
    const summary = normalizedText(turn.asked.summary);
    if (!summary) return false;
    const fullPrompt = normalizedText(promptContent(turn.asked));
    return summary !== fullPrompt;
  });
  const visibleCount = summarizedTurns.length > 10 ? 3 : summarizedTurns.length > 5 ? 2 : 1;
  const visibleTurns = new Set(
    summarizedTurns
      .map((turn, index) => ({
        turn,
        score: selectionScore(`${seed}:${turn.asked.id ?? index}:${normalizedText(turn.asked.summary)}`)
      }))
      .sort((left, right) => left.score - right.score)
      .slice(0, visibleCount)
      .map(({ turn }) => turn)
  );

  const items: PublicTranscriptItem[] = [{ kind: 'turn', turn: turns[firstPromptIndex] }];
  let hiddenPromptCount = 0;
  let hiddenReplyCount = 0;
  let hiddenChangedFileCount = 0;

  function flushHidden() {
    if (!hiddenPromptCount) return;
    items.push({
      kind: 'hidden',
      promptCount: hiddenPromptCount,
      replyCount: hiddenReplyCount,
      changedFileCount: hiddenChangedFileCount
    });
    hiddenPromptCount = 0;
    hiddenReplyCount = 0;
    hiddenChangedFileCount = 0;
  }

  for (const turn of remainingTurns) {
    if (visibleTurns.has(turn)) {
      flushHidden();
      items.push({ kind: 'turn', turn });
      continue;
    }
    hiddenPromptCount += 1;
    hiddenReplyCount += turn.replyCount;
    hiddenChangedFileCount += turn.changedFileCount;
  }
  flushHidden();

  return items;
}
