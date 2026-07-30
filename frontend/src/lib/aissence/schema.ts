import { z } from 'zod';

const MAX_COMMITS = 5_000;

const unsignedInteger = z.number().int().nonnegative();
const optionalDate = z
  .string()
  .refine((value) => !Number.isNaN(Date.parse(value)), 'Must be an RFC 3339 date')
  .optional();
const nullableDate = z
  .string()
  .refine((value) => !Number.isNaN(Date.parse(value)), 'Must be an RFC 3339 date')
  .nullable();

export const tokenCountSchema = z
  .object({
    input: unsignedInteger,
    output: unsignedInteger,
    cache_read: unsignedInteger.optional(),
    cache_create: unsignedInteger.optional()
  })
  .strip();

export const changeSchema = z
  .object({
    tool: z.string(),
    file: z.string(),
    diff: z.string()
  })
  .strip();

export const agentSchema = z
  .object({
    provider: z.string().min(1),
    model: z.string().nullable(),
    tokens: tokenCountSchema.optional()
  })
  .strip();

export const messageSchema = z
  .object({
    kind: z.enum(['UserWrote', 'AiWrote', 'AiChanged']),
    summary: z.string().optional(),
    content: z.string().optional(),
    changes: z.array(changeSchema),
    timestamp: nullableDate,
    tokens: tokenCountSchema.optional()
  })
  .strip()
  .superRefine((message, context) => {
    if (message.kind === 'UserWrote' && message.summary === undefined) {
      context.addIssue({
        code: 'custom',
        path: ['summary'],
        message: 'Every UserWrote message must contain a summary'
      });
    }
    if (message.kind !== 'UserWrote' && message.summary !== undefined) {
      context.addIssue({
        code: 'custom',
        path: ['summary'],
        message: 'Only UserWrote messages contain per-message summaries'
      });
    }
    if (message.kind !== 'AiChanged' && message.changes.length > 0) {
      context.addIssue({
        code: 'custom',
        path: ['changes'],
        message: 'Only AiChanged messages may contain changes'
      });
    }
    if (message.kind === 'AiChanged' && message.content !== undefined) {
      context.addIssue({
        code: 'custom',
        path: ['content'],
        message: 'AiChanged messages do not contain retained text'
      });
    }
    if (message.kind !== 'AiChanged' && message.content === undefined) {
      context.addIssue({
        code: 'custom',
        path: ['content'],
        message: 'UserWrote and AiWrote messages must contain retained text'
      });
    }
  });

export const commitStatsSchema = z
  .object({
    files_changed: unsignedInteger,
    lines_added: unsignedInteger,
    lines_deleted: unsignedInteger
  })
  .strip();

export const commitSummarySchema = z
  .object({
    user_requested: z.string(),
    ai_answered: z.string(),
    ai_implemented: z.string(),
    messages: z.array(messageSchema),
    agents: z.array(agentSchema).optional().default([]),
    meta: z.string().optional().default(''),
    tokens: tokenCountSchema.optional()
  })
  .strip();

export const commitEntrySchema = z
  .object({
    signature: z.unknown(),
    title: z.string().optional(),
    date: optionalDate,
    stats: commitStatsSchema.optional(),
    summary: commitSummarySchema
  })
  .strip();

export const aissenceFileV1Schema = z
  .object({
    version: z.literal(1),
    commits: z.record(z.string(), commitEntrySchema),
    ignored: z.array(z.string()).optional().default([])
  })
  .strip()
  .superRefine((file, context) => {
    const entries = Object.entries(file.commits);
    if (entries.length > MAX_COMMITS) {
      context.addIssue({
        code: 'too_big',
        origin: 'object',
        maximum: MAX_COMMITS,
        inclusive: true,
        path: ['commits'],
        message: `A file may contain at most ${MAX_COMMITS.toLocaleString()} commits`
      });
    }
    for (const [hash] of entries) {
      if (!/^[0-9a-f]{40}$/.test(hash)) {
        context.addIssue({
          code: 'custom',
          path: ['commits', hash],
          message: 'Commit keys must be lowercase 40-character SHA-1 hashes'
        });
      }
    }
    file.ignored.forEach((hash, index) => {
      if (!/^[0-9a-f]{40}$/.test(hash)) {
        context.addIssue({
          code: 'custom',
          path: ['ignored', index],
          message: 'Ignored commits must be lowercase 40-character SHA-1 hashes'
        });
      }
    });
  });

export type TokenCount = z.infer<typeof tokenCountSchema>;
export type Change = z.infer<typeof changeSchema>;
export type Agent = z.infer<typeof agentSchema>;
export type AissenceMessage = z.infer<typeof messageSchema>;
export type CommitStats = z.infer<typeof commitStatsSchema>;
export type CommitSummary = z.infer<typeof commitSummarySchema>;
export type CommitEntry = z.infer<typeof commitEntrySchema>;
export type AissenceFileV1 = z.infer<typeof aissenceFileV1Schema>;

export interface NormalizedCommit extends CommitEntry {
  hash: string;
}

export interface NormalizedSnapshot {
  commits: NormalizedCommit[];
}

export interface ParsedAissenceFile {
  file: AissenceFileV1;
  snapshot: NormalizedSnapshot;
  warnings: string[];
}

function fallbackTimestamp(entry: CommitEntry): number {
  if (entry.date) return Date.parse(entry.date);
  const timestamps = entry.summary.messages
    .map((message) => (message.timestamp ? Date.parse(message.timestamp) : Number.NaN))
    .filter(Number.isFinite);
  return timestamps.length > 0 ? Math.max(...timestamps) : Number.POSITIVE_INFINITY;
}

export function parseAissence(input: string): ParsedAissenceFile {
  let json: unknown;
  try {
    json = JSON.parse(input);
  } catch {
    throw new AissenceValidationError(['The file is not valid JSON.']);
  }

  const result = aissenceFileV1Schema.safeParse(json);
  if (!result.success) {
    throw new AissenceValidationError(
      result.error.issues.slice(0, 25).map((issue) => {
        const path = issue.path.length ? issue.path.join('.') : 'file';
        return `${path}: ${issue.message}`;
      })
    );
  }

  const ignored = new Set(result.data.ignored);
  const warnings: string[] = [];
  const commits = Object.entries(result.data.commits)
    .filter(([hash]) => !ignored.has(hash))
    .map(([hash, entry]) => {
      if (!entry.title) warnings.push(`${hash.slice(0, 10)} has no commit title.`);
      if (!entry.date) warnings.push(`${hash.slice(0, 10)} has no commit date.`);
      if (!entry.stats) warnings.push(`${hash.slice(0, 10)} has no commit statistics.`);
      return { hash, ...entry };
    })
    .sort((left, right) => {
      const byTime = fallbackTimestamp(left) - fallbackTimestamp(right);
      return byTime || left.hash.localeCompare(right.hash);
    });

  return {
    file: result.data,
    snapshot: { commits },
    warnings
  };
}

export class AissenceValidationError extends Error {
  readonly issues: string[];

  constructor(issues: string[]) {
    super(issues[0] ?? 'The file is not valid.');
    this.name = 'AissenceValidationError';
    this.issues = issues;
  }
}
