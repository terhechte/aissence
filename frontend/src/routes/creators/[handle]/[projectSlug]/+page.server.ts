import { error } from '@sveltejs/kit';
import { groupTranscript } from '$lib/ledger';
import {
  estimatePotentialCost,
  fetchOpenRouterCatalog
} from '$lib/server/openrouter-pricing';
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
    const { data: subscription } = await locals.supabase
      .from('creator_subscriptions')
      .select('status,current_period_end')
      .eq('subscriber_id', claims.sub)
      .eq('creator_id', profile.id)
      .maybeSingle();
    isEntitled = Boolean(
      subscription &&
        ['active', 'trialing'].includes(subscription.status) &&
        (!subscription.current_period_end || Date.parse(subscription.current_period_end) > Date.now())
    );
  }

  const [{ data: screenshots }, { data: commits }] = await Promise.all([
    locals.supabase
      .from('project_screenshots')
      .select('*')
      .eq('project_id', project.id)
      .order('sort_order'),
    locals.supabase
      .from('commits')
      .select(
        '*,commit_agents(*),messages(*,message_sources(content),changes(tool,file_path,diff,sort_order))'
      )
      .eq('project_id', project.id)
      .order('sort_order')
  ]);

  const signed = screenshots?.length
    ? await locals.supabase.storage
        .from('project-media')
        .createSignedUrls(screenshots.map((shot) => shot.storage_path), 60 * 60)
    : { data: [] };

  const normalizedCommits = (commits ?? []).map((commit) => ({
    ...commit,
    total_prompt_count: (commit.messages ?? []).filter((message: any) => message.kind === 'UserWrote').length,
    commit_agents: [...(commit.commit_agents ?? [])].sort((a, b) => a.sort_order - b.sort_order),
    messages: [...(commit.messages ?? [])]
      .sort((a, b) => a.sort_order - b.sort_order)
      .map((message) => ({
        ...message,
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
  const visibleCommits = isEntitled
    ? pricedCommits
    : pricedCommits.map((commit) => ({
        ...commit,
        messages: groupTranscript(commit.messages).flatMap((turn) =>
          turn.asked
            ? [
                {
                  ...turn.asked,
                  following_reply_count: turn.replyCount,
                  following_changed_file_count: turn.changedFileCount
                }
              ]
            : []
        )
      }));

  return {
    profile,
    project,
    isOwner,
    isEntitled,
    screenshots: (screenshots ?? []).map((shot, index) => ({
      ...shot,
      url: signed.data?.[index]?.signedUrl ?? ''
    })),
    commits: visibleCommits
  };
};
