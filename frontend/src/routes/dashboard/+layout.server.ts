import { redirect } from '@sveltejs/kit';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = async ({ locals }) => {
  const { claims } = await locals.safeGetClaims();
  if (!claims) redirect(303, '/auth');
  return { claims };
};
