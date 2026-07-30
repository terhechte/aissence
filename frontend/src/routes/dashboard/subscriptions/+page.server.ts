import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const { data: subscriptions } = await supabase
    .from('creator_subscriptions')
    .select('creator_id,status,current_period_end')
    .eq('subscriber_id', claims.sub)
    .in('status', ['active', 'trialing']);

  const activeSubscriptions = (subscriptions ?? []).filter(
    (subscription) =>
      !subscription.current_period_end || Date.parse(subscription.current_period_end) > Date.now()
  );
  const creatorIds = activeSubscriptions.map((subscription) => subscription.creator_id);
  if (!creatorIds.length) return { ledgers: [] };

  const [{ data: profiles }, { data: projects }] = await Promise.all([
    supabase.from('profiles').select('id,handle,display_name').in('id', creatorIds),
    supabase
      .from('projects')
      .select('id,slug,name,tagline,owner_id,updated_at')
      .in('owner_id', creatorIds)
      .eq('visibility', 'published')
      .order('updated_at', { ascending: false })
  ]);
  const profilesById = new Map((profiles ?? []).map((profile) => [profile.id, profile]));

  return {
    ledgers: (projects ?? [])
      .map((project) => {
        const creator = profilesById.get(project.owner_id);
        if (!creator) return null;
        return {
          ...project,
          creator
        };
      })
      .filter((project): project is NonNullable<typeof project> => project !== null)
  };
};
