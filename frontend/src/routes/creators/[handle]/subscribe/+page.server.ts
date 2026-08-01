import { error, redirect } from '@sveltejs/kit';
import { subscriptionGrantsAccess } from '$lib/server/subscriptions';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params, url }) => {
  const { claims } = await locals.safeGetClaims();
  if (!claims) redirect(303, `/auth?next=${encodeURIComponent(url.pathname)}`);
  const { data: profile } = await locals.supabase!
    .from('profiles')
    .select('id,handle,display_name,bio')
    .eq('handle', params.handle)
    .maybeSingle();
  if (!profile) error(404, 'Creator not found');
  const [{ data: subscription }, { data: plans }, { data: paymentsReady }] = await Promise.all([
    locals.supabase!
      .from('creator_subscriptions')
      .select('status,current_period_end,source,cancel_at_period_end,plan_id')
      .eq('subscriber_id', claims.sub)
      .eq('creator_id', profile.id)
      .maybeSingle(),
    locals.supabase!
      .from('creator_plans')
      .select('id,name,description,interval,price_cents,currency,active')
      .eq('creator_id', profile.id)
      .eq('active', true)
      .order('interval'),
    locals.supabase!.rpc('creator_payments_ready', { p_creator_id: profile.id })
  ]);
  return {
    profile,
    subscription,
    plans: plans ?? [],
    paymentsReady: paymentsReady === true,
    isEntitled: subscriptionGrantsAccess(subscription)
  };
};
