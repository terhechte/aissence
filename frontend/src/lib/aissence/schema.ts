import { z } from 'zod';

const MAX_COMMITS = 5_000;
const unsignedInteger = z.number().int().nonnegative();
const rfc3339 = z.string().refine((value) => !Number.isNaN(Date.parse(value)), 'Must be an RFC 3339 date');
const nullableDate = rfc3339.nullable();

export const tokenCountSchema = z.object({
  input: unsignedInteger,
  output: unsignedInteger,
  cache_read: unsignedInteger.optional(),
  cache_create: unsignedInteger.optional()
});

export const changeSchema = z.object({ tool: z.string(), file: z.string(), diff: z.string() });

export const agentSchema = z.object({
  provider: z.string().min(1),
  model: z.string().nullable(),
  tokens: tokenCountSchema.optional()
});

const messageShape = {
  kind: z.enum(['UserWrote', 'AiWrote', 'AiChanged']),
  content: z.string().optional(),
  changes: z.array(changeSchema),
  timestamp: nullableDate,
  tokens: tokenCountSchema.optional()
};

function validateMessage(
  message: { kind: string; content?: string; changes: unknown[]; subscriber_summary?: string },
  context: z.RefinementCtx,
) {
  if (message.kind !== 'AiChanged' && message.changes.length > 0) {
    context.addIssue({ code: 'custom', path: ['changes'], message: 'Only AiChanged messages may contain changes' });
  }
  if (message.kind === 'AiChanged' && message.content !== undefined) {
    context.addIssue({ code: 'custom', path: ['content'], message: 'AiChanged messages do not contain retained text' });
  }
  if (message.kind !== 'AiChanged' && message.content === undefined) {
    context.addIssue({ code: 'custom', path: ['content'], message: 'Text messages must contain retained text' });
  }
  if (message.kind !== 'UserWrote' && message.subscriber_summary !== undefined) {
    context.addIssue({ code: 'custom', path: ['subscriber_summary'], message: 'Only user prompts have subscriber summaries' });
  }
}

export const sourceMessageSchema = z.object(messageShape).strict().superRefine(validateMessage);
export const messageSchema = z
  .object({ ...messageShape, subscriber_summary: z.string().min(1).optional() })
  .strict()
  .superRefine(validateMessage);

export const commitStatsSchema = z.object({
  files_changed: unsignedInteger,
  lines_added: unsignedInteger,
  lines_deleted: unsignedInteger
});

export const publicBeatSchema = z.object({
  message_index: unsignedInteger,
  label: z.string().trim().min(1),
  text: z.string().trim().min(1)
});

export const commitSummarySchema = z.object({
  summarizer_version: unsignedInteger.default(1),
  public: z.object({
    subline: z.string().trim().max(1000).default(''),
    summary: z.string().trim().min(1),
    beats: z.array(publicBeatSchema).max(3),
    origin: z.enum(['model', 'metadata_fallback', 'parse_fallback', 'privacy_fallback']).default('model')
  }),
  messages: z.array(messageSchema),
  agents: z.array(agentSchema).default([]),
  tokens: tokenCountSchema.optional()
});

export const commitEntrySchema = z.object({
  commit_hash: z.string().regex(/^[0-9a-f]{40}$/),
  source_digest: z.string().regex(/^sha256:[0-9a-f]{64}$/),
  sanitizer_version: unsignedInteger,
  title: z.string(),
  body: z.string().optional(),
  date: rfc3339,
  stats: commitStatsSchema,
  files: z.array(z.object({
    status: z.enum(['Added', 'Modified', 'Deleted', 'Renamed', 'Copied']),
    path: z.string(),
    previous_path: z.string().nullable()
  })).default([]),
  source: z.object({
    messages: z.array(sourceMessageSchema),
    agents: z.array(agentSchema).default([]),
    tokens: tokenCountSchema.optional()
  }),
  summary: commitSummarySchema.optional()
}).strict().superRefine((commit, context) => {
  if (commit.summary && commit.summary.messages.length !== commit.source.messages.length) {
    context.addIssue({ code: 'custom', path: ['summary', 'messages'], message: 'Summary and source message counts must match' });
  }
  for (const [index, beat] of commit.summary?.public.beats.entries() ?? []) {
    if (commit.source.messages[beat.message_index]?.kind !== 'UserWrote') {
      context.addIssue({ code: 'custom', path: ['summary', 'public', 'beats', index, 'message_index'], message: 'A public beat must reference a user prompt' });
    }
  }
});

export const aissenceProjectSchema = z.object({
  source_id: z.string().uuid(),
  name: z.string().trim().min(1).max(120).optional(),
  repository: z.string().trim().min(1).max(240).optional()
});

export const aissenceFileV2Schema = z.object({
  version: z.literal(2),
  project: aissenceProjectSchema,
  commits: z.array(commitEntrySchema).max(MAX_COMMITS),
  ignored: z.array(z.string().regex(/^[0-9a-f]{40}$/)).default([])
}).strict().superRefine((file, context) => {
  const hashes = new Set<string>();
  file.commits.forEach((commit, index) => {
    if (hashes.has(commit.commit_hash)) {
      context.addIssue({ code: 'custom', path: ['commits', index, 'commit_hash'], message: 'Commit hashes must be unique' });
    }
    hashes.add(commit.commit_hash);
  });
});

export type TokenCount = z.infer<typeof tokenCountSchema>;
export type Change = z.infer<typeof changeSchema>;
export type Agent = z.infer<typeof agentSchema>;
export type AissenceMessage = z.infer<typeof messageSchema>;
export type CommitStats = z.infer<typeof commitStatsSchema>;
export type CommitSummary = z.infer<typeof commitSummarySchema>;
export type CommitEntry = z.infer<typeof commitEntrySchema>;
export type AissenceProject = z.infer<typeof aissenceProjectSchema>;
export type AissenceFileV2 = z.infer<typeof aissenceFileV2Schema>;
export type NormalizedCommit = CommitEntry;
export interface NormalizedSnapshot { commits: NormalizedCommit[] }
export interface ParsedAissenceFile {
  file: AissenceFileV2;
  snapshot: NormalizedSnapshot;
  warnings: string[];
}

export function parseAissence(input: string): ParsedAissenceFile {
  let json: unknown;
  try {
    json = JSON.parse(input);
  } catch {
    throw new AissenceValidationError(['The file is not valid JSON.']);
  }
  const result = aissenceFileV2Schema.safeParse(json);
  if (!result.success) {
    throw new AissenceValidationError(result.error.issues.slice(0, 25).map((issue) => {
      const path = issue.path.length ? issue.path.join('.') : 'file';
      return `${path}: ${issue.message}`;
    }));
  }
  const ignored = new Set(result.data.ignored);
  const commits = result.data.commits
    .filter((commit) => !ignored.has(commit.commit_hash))
    .sort((left, right) => Date.parse(left.date) - Date.parse(right.date) || left.commit_hash.localeCompare(right.commit_hash));
  const warnings = commits.flatMap((commit) =>
    commit.sanitizer_version !== 2
      ? [`${commit.commit_hash.slice(0, 10)} uses an unsupported sanitizer version.`]
      : []
  );
  return { file: result.data, snapshot: { commits }, warnings };
}

export class AissenceValidationError extends Error {
  readonly issues: string[];
  constructor(issues: string[]) {
    super(issues[0] ?? 'The file is not valid.');
    this.name = 'AissenceValidationError';
    this.issues = issues;
  }
}
