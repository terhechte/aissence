import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const { data: profile } = await locals.supabase!
    .from('profiles')
    .select('id')
    .eq('id', claims.sub)
    .maybeSingle();
  if (!profile) redirect(303, '/dashboard');
  return {};
};
