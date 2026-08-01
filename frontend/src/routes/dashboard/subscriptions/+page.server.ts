import { redirect } from '@sveltejs/kit';
import { subscriptionGrantsAccess } from '$lib/server/subscriptions';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const { data: subscriptions } = await supabase
    .from('creator_subscriptions')
    .select('id,creator_id,status,current_period_end,source,plan_id,cancel_at_period_end,canceled_at')
    .eq('subscriber_id', claims.sub)
    .order('updated_at', { ascending: false });

  const activeSubscriptions = (subscriptions ?? []).filter((subscription) =>
    subscriptionGrantsAccess(subscription)
  );
  const creatorIds = activeSubscriptions.map((subscription) => subscription.creator_id);
  const allCreatorIds = [...new Set((subscriptions ?? []).map((subscription) => subscription.creator_id))];
  if (!allCreatorIds.length) return { ledgers: [], subscriptions: [] };

  const [{ data: profiles }, { data: projects }, { data: plans }] = await Promise.all([
    supabase.from('profiles').select('id,handle,display_name').in('id', allCreatorIds),
    supabase
      .from('projects')
      .select('id,slug,name,tagline,owner_id,updated_at')
      .in('owner_id', creatorIds)
      .eq('visibility', 'published')
      .order('updated_at', { ascending: false }),
    supabase
      .from('creator_plans')
      .select('id,name,interval,price_cents,currency')
      .in('creator_id', allCreatorIds)
  ]);
  const profilesById = new Map((profiles ?? []).map((profile) => [profile.id, profile]));
  const plansById = new Map((plans ?? []).map((plan) => [plan.id, plan]));

  return {
    subscriptions: (subscriptions ?? []).map((subscription) => ({
      ...subscription,
      access: subscriptionGrantsAccess(subscription),
      creator: profilesById.get(subscription.creator_id) ?? null,
      plan: subscription.plan_id ? plansById.get(subscription.plan_id) ?? null : null
    })),
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
