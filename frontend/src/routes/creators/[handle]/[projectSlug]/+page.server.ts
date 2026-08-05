import { error } from '@sveltejs/kit';
import {
  estimatePotentialCost,
  fetchOpenRouterCatalog
} from '$lib/server/openrouter-pricing';
import { hasCreatorAccess } from '$lib/server/subscriptions';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params, setHeaders }) => {
  if (!locals.supabase) error(503, 'Supabase is not configured');
  setHeaders({
    'cache-control': 'private, no-store, max-age=0',
    vary: 'Cookie'
  });

  const { claims } = await locals.safeGetClaims();
  const { data: profile } = await locals.supabase
    .from('profiles')
    .select('id,handle,display_name,bio')
    .eq('handle', params.handle)
    .maybeSingle();
  if (!profile) error(404, 'Creator not found');

  const { data: project } = await locals.supabase
    .from('projects')
    .select('*')
    .eq('owner_id', profile.id)
    .eq('slug', params.projectSlug)
    .maybeSingle();
  if (!project) error(404, 'Project not found');

  const isOwner = claims?.sub === profile.id;
  let isEntitled = isOwner;
  if (claims && !isOwner) {
    isEntitled = await hasCreatorAccess(locals.supabase, claims.sub, profile.id);
  }

  const publicCommitFields = [
    'id', 'commit_hash', 'title', 'committed_at', 'files_changed', 'lines_added',
    'lines_deleted', 'public_story', 'public_beats', 'prompt_count', 'reply_count',
    'change_message_count', 'token_input', 'token_output', 'token_cache_read',
    'token_cache_create', 'sort_order', 'potential_cost_usd', 'input_cost_usd',
    'output_cost_usd', 'pricing_model_ids', 'pricing_calculated_at', 'summary_status',
    'public_prompt_summaries', 'commit_agents(*)'
  ].join(',');
  const publicPromptFields = `${publicCommitFields},messages(id,kind,sort_order,message_summaries(summary))`;
  const entitledCommitFields = `${publicCommitFields},messages(id,kind,occurred_at,token_input,token_output,token_cache_read,token_cache_create,sort_order,changed_file_count,message_summaries(summary),message_sources(content),changes(tool,file_path,diff,sort_order))`;
  const [{ data: screenshots }, { data: commits }] = await Promise.all([
    locals.supabase
      .from('project_screenshots')
      .select('*')
      .eq('project_id', project.id)
      .order('sort_order'),
    locals.supabase
      .from('commits')
      .select(isEntitled ? entitledCommitFields : publicPromptFields)
      .eq('project_id', project.id)
      .order('sort_order')
  ]);

  const signed = screenshots?.length
    ? await locals.supabase.storage
        .from('project-media')
        .createSignedUrls(screenshots.map((shot) => shot.storage_path), 60 * 60)
    : { data: [] };

  const normalizedCommits = ((commits ?? []) as any[]).map((commit) => ({
    ...commit,
    total_prompt_count: Number(commit.prompt_count ?? 0),
    commit_agents: [...(commit.commit_agents ?? [])].sort((a, b) => a.sort_order - b.sort_order),
    messages: [...(commit.messages ?? [])]
      .sort((a, b) => a.sort_order - b.sort_order)
      .map((message) => ({
        ...message,
        subscriber_summary: Array.isArray(message.message_summaries)
          ? message.message_summaries[0]?.summary ?? null
          : message.message_summaries?.summary ?? null,
        changes: [...(message.changes ?? [])].sort((a, b) => a.sort_order - b.sort_order)
      }))
  }));
  const catalog = normalizedCommits.some(
    (commit) => commit.token_input !== null && commit.potential_cost_usd === null
  )
    ? await fetchOpenRouterCatalog()
    : [];
  const pricedCommits = normalizedCommits.map((commit) => {
    if (commit.potential_cost_usd !== null || commit.token_input === null) return commit;
    const pricing = estimatePotentialCost(
      {
        input: Number(commit.token_input ?? 0),
        output: Number(commit.token_output ?? 0),
        ...(commit.token_cache_read === null
          ? {}
          : { cache_read: Number(commit.token_cache_read) }),
        ...(commit.token_cache_create === null
          ? {}
          : { cache_create: Number(commit.token_cache_create) })
      },
      commit.commit_agents.map((agent: any) => ({
        provider: agent.provider,
        model: agent.model
      })),
      catalog
    );
    return {
      ...commit,
      input_cost_usd: pricing.input_cost_usd,
      output_cost_usd: pricing.output_cost_usd,
      potential_cost_usd: pricing.potential_cost_usd,
      pricing_model_ids: pricing.model_ids,
      pricing_calculated_at: pricing.priced_at
    };
  });
  return {
    profile,
    project,
    isOwner,
    isEntitled,
    screenshots: (screenshots ?? []).map((shot, index) => ({
      ...shot,
      url: signed.data?.[index]?.signedUrl ?? ''
    })),
    commits: pricedCommits
  };
};
