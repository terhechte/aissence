export interface TranscriptTurn {
  asked: any | null;
  activity: any[];
  replyCount: number;
  changedFileCount: number;
}

function messageChangedFileCount(message: any): number {
  if (message.changed_file_count !== undefined && message.changed_file_count !== null) {
    const persisted = Number(message.changed_file_count);
    if (persisted > 0 || !(message.changes ?? []).length) return persisted;
  }
  return new Set((message.changes ?? []).map((change: any) => change.file_path)).size;
}

/** Group an entitled ledger's assistant activity under each user prompt. */
export function groupTranscript(messages: any[]): TranscriptTurn[] {
  const turns: TranscriptTurn[] = [];
  let current: TranscriptTurn | null = null;
  for (const message of messages) {
    if (message.kind === 'UserWrote') {
      current = { asked: message, activity: [], replyCount: 0, changedFileCount: 0 };
      turns.push(current);
      continue;
    }
    if (!current) {
      current = { asked: null, activity: [], replyCount: 0, changedFileCount: 0 };
      turns.push(current);
    }
    current.activity.push(message);
    if (message.kind === 'AiWrote') current.replyCount += 1;
    if (message.kind === 'AiChanged') current.changedFileCount += messageChangedFileCount(message);
  }
  return turns;
}
