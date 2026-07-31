const APPSHOT_BLOCK =
  /<appshot\b(?=[^>]*\bapp\s*=\s*(?:"([^"]*)"|'([^']*)'))[^>]*(?:\/>|>[\s\S]*?<\/appshot\s*>)/gi;
const HIDDEN_NOISE_TAGS = [
  'fork-boilerplate',
  'task-notification',
  'command-name',
  'command-message',
  'command-args',
  'local-command-stdout',
  'usage',
  'file',
  'packetEnd'
] as const;
const HIDDEN_NOISE_BLOCK = new RegExp(
  `[ \\t]*<(${HIDDEN_NOISE_TAGS.join('|')})\\b[^>]*(?:\\/>|>[\\s\\S]*?<\\/\\1\\s*>|>)[ \\t]*(?:\\r?\\n|$)`,
  'gi'
);
const HIDDEN_NOISE_INLINE_BLOCK = new RegExp(
  `<(${HIDDEN_NOISE_TAGS.join('|')})\\b[^>]*(?:>[\\s\\S]*?<\\/\\1\\s*>|\\/?>)`,
  'gi'
);
const HIDDEN_NOISE_CLOSING_TAG = new RegExp(
  `<\\/(${HIDDEN_NOISE_TAGS.join('|')})\\s*>`,
  'gi'
);

function escapeMarkdownText(value: string) {
  return value.replace(/[\\`*_[\]<>]/g, '\\$&');
}

export function filterPromptNoise(content: string) {
  const withAppshotsFiltered = content.replace(
    APPSHOT_BLOCK,
    (_match, doubleQuotedApp, singleQuotedApp) => {
      const app = String(doubleQuotedApp ?? singleQuotedApp ?? '').trim();
      return `User Included an appshot${app ? ` of ${escapeMarkdownText(app)}` : ''}`;
    }
  );

  return withAppshotsFiltered
    .replace(HIDDEN_NOISE_BLOCK, '')
    .replace(HIDDEN_NOISE_INLINE_BLOCK, '')
    .replace(HIDDEN_NOISE_CLOSING_TAG, '');
}
