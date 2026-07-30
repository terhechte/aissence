import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = async ({ locals }) => {
  const { claims } = await locals.safeGetClaims();
  return { claims };
};
