import { parseAissence, type AissenceFileV2, type TokenCount } from '$lib/aissence/schema';
import { repositoryFields } from '$lib/aissence/project';

const PREVIEW_TTL_MS = 24 * 60 * 60 * 1000;
const MAX_PREVIEWS = 10;

export interface DevPreviewData {
  profile: { id: string; handle: string; display_name: string; bio: null };
  project: {
    id: string;
    name: string;
    tagline: string;
    description: string;
    product_url: null;
    repository_label: string;
    repository_url: string | null;
    visibility: 'draft';
  };
  commits: any[];
  screenshots: never[];
  warnings: string[];
}

interface StoredPreview {
  createdAt: number;
  data: DevPreviewData;
}

const globalState = globalThis as typeof globalThis & {
  __aissenceDevPreviews?: Map<string, StoredPreview>;
};

const previews = (globalState.__aissenceDevPreviews ??= new Map());

function tokenFields(tokens: TokenCount | undefined) {
  return {
    token_input: tokens?.input ?? null,
    token_output: tokens?.output ?? null,
    token_cache_read: tokens?.cache_read ?? null,
    token_cache_create: tokens?.cache_create ?? null
  };
}

function previewProject(file: AissenceFileV2) {
  const repository = file.project.repository ? repositoryFields(file.project.repository) : null;
  const fallbackName = repository?.label.split('/').at(-1) || 'Local preview';
  return {
    id: file.project.source_id,
    name: file.project.name || fallbackName,
    tagline: '',
    description: 'Rendered directly from a locally generated .aissence file.',
    product_url: null,
    repository_label: repository?.label ?? '',
    repository_url: repository?.url || null,
    visibility: 'draft' as const
  };
}

export function previewDataFromDocument(input: string): DevPreviewData {
  const parsed = parseAissence(input);
  return {
    profile: {
      id: 'local-preview',
      handle: 'local-preview',
      display_name: 'Local preview',
      bio: null
    },
    project: previewProject(parsed.file),
    screenshots: [],
    warnings: parsed.warnings,
    commits: parsed.snapshot.commits.map((commit, sortOrder) => {
      const summary = commit.summary;
      const promptCount = commit.source.messages.filter(
        (message) => message.kind === 'UserWrote' && message.content?.trim()
      ).length;
      const replyCount = commit.source.messages.filter((message) => message.kind === 'AiWrote').length;
      const changeMessageCount = commit.source.messages.filter((message) => message.kind === 'AiChanged').length;
      const summarizedPrompts = summary?.messages.filter(
        (message) => message.kind === 'UserWrote' && message.subscriber_summary
      ).length ?? 0;
      const tokens = summary?.tokens ?? commit.source.tokens;

      return {
        id: commit.commit_hash,
        commit_hash: commit.commit_hash,
        title: commit.title,
        committed_at: commit.date,
        files_changed: commit.stats.files_changed,
        lines_added: commit.stats.lines_added,
        lines_deleted: commit.stats.lines_deleted,
        public_story: summary?.public.summary ?? '',
        public_beats: summary?.public.beats ?? [],
        prompt_count: promptCount,
        total_prompt_count: promptCount,
        reply_count: replyCount,
        change_message_count: changeMessageCount,
        public_prompt_summaries: promptCount > 0 && summarizedPrompts === promptCount,
        sort_order: sortOrder,
        summary_status: summary ? 'succeeded' : 'skipped',
        summary_origin: summary?.public.origin ?? 'pending',
        summarizer_version: summary?.summarizer_version ?? null,
        summary_error: null,
        input_cost_usd: null,
        output_cost_usd: null,
        potential_cost_usd: null,
        pricing_model_ids: [],
        pricing_calculated_at: null,
        ...tokenFields(tokens),
        commit_agents: commit.source.agents.map((agent, agentOrder) => ({
          ...agent,
          sort_order: agentOrder
        })),
        messages: commit.source.messages.map((message, messageOrder) => {
          const generated = summary?.messages[messageOrder];
          const messageTokens = generated?.tokens ?? message.tokens;
          return {
            id: `${commit.commit_hash}-${messageOrder}`,
            kind: message.kind,
            occurred_at: message.timestamp,
            sort_order: messageOrder,
            changed_file_count: new Set(message.changes.map((change) => change.file)).size,
            subscriber_summary: generated?.subscriber_summary ?? null,
            message_sources: message.content === undefined ? [] : [{ content: message.content }],
            changes: message.changes.map((change, changeOrder) => ({
              tool: change.tool,
              file_path: change.file,
              diff: change.diff,
              sort_order: changeOrder
            })),
            ...tokenFields(messageTokens)
          };
        })
      };
    })
  };
}

function prunePreviews(now: number) {
  for (const [id, preview] of previews) {
    if (now - preview.createdAt > PREVIEW_TTL_MS) previews.delete(id);
  }
  while (previews.size >= MAX_PREVIEWS) {
    const oldest = previews.keys().next().value;
    if (!oldest) break;
    previews.delete(oldest);
  }
}

export function saveDevPreview(input: string): string {
  const now = Date.now();
  prunePreviews(now);
  const id = crypto.randomUUID();
  previews.set(id, { createdAt: now, data: previewDataFromDocument(input) });
  return id;
}

export function getDevPreview(id: string): DevPreviewData | null {
  const preview = previews.get(id);
  if (!preview) return null;
  if (Date.now() - preview.createdAt > PREVIEW_TTL_MS) {
    previews.delete(id);
    return null;
  }
  return preview.data;
}
