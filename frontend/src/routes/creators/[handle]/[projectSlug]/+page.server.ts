import { error } from '@sveltejs/kit';
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

  return {
    profile,
    project,
    isOwner,
    isEntitled,
    screenshots: (screenshots ?? []).map((shot, index) => ({
      ...shot,
      url: signed.data?.[index]?.signedUrl ?? ''
    })),
    commits: (commits ?? []).map((commit) => ({
      ...commit,
      commit_agents: [...(commit.commit_agents ?? [])].sort((a, b) => a.sort_order - b.sort_order),
      messages: [...(commit.messages ?? [])]
        .sort((a, b) => a.sort_order - b.sort_order)
        .map((message) => ({
          ...message,
          changes: [...(message.changes ?? [])].sort((a, b) => a.sort_order - b.sort_order)
        }))
    }))
  };
};
