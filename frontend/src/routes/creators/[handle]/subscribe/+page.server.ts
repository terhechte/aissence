import { error, redirect } from '@sveltejs/kit';
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
  const { data: subscription } = await locals.supabase!
    .from('creator_subscriptions')
    .select('status,current_period_end,source')
    .eq('subscriber_id', claims.sub)
    .eq('creator_id', profile.id)
    .maybeSingle();
  return { profile, subscription };
};
