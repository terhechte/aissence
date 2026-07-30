import { error, redirect, type RequestEvent } from '@sveltejs/kit';

export async function requireClaims(event: Pick<RequestEvent, 'locals'>) {
  const { claims } = await event.locals.safeGetClaims();
  if (!claims) redirect(303, '/auth');
  return claims;
}

export async function requireApiClaims(event: Pick<RequestEvent, 'locals'>) {
  const { claims } = await event.locals.safeGetClaims();
  if (!claims) error(401, 'Authentication required');
  if (!event.locals.supabase) error(503, 'Supabase is not configured');
  return claims;
}

export async function requireProjectOwner(
  event: Pick<RequestEvent, 'locals'>,
  projectId: string
) {
  const claims = await requireApiClaims(event);
  const { data } = await event.locals.supabase!
    .from('projects')
    .select('id,owner_id')
    .eq('id', projectId)
    .eq('owner_id', claims.sub)
    .maybeSingle();
  if (!data) error(404, 'Project not found');
  return claims;
}
