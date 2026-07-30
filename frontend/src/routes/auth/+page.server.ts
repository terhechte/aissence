import { fail, redirect } from '@sveltejs/kit';
import { env } from '$env/dynamic/public';
import { env as privateEnv } from '$env/dynamic/private';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, url }) => {
  const { claims } = await locals.safeGetClaims();
  if (claims) redirect(303, '/dashboard');
  const next = url.searchParams.get('next');
  return { configured: Boolean(locals.supabase), next: next?.startsWith('/') ? next : '/dashboard' };
};

export const actions: Actions = {
  default: async ({ request, locals, url }) => {
    if (!locals.supabase) {
      return fail(503, { error: 'Supabase is not configured yet.' });
    }
    const form = await request.formData();
    const email = String(form.get('email') ?? '').trim();
    const next = String(form.get('next') ?? '/dashboard');
    if (!/.+@.+\..+/.test(email)) return fail(400, { email, error: 'Enter a valid email address.' });

    const siteUrl = env.PUBLIC_SITE_URL || privateEnv.PUBLIC_SITE_URL || url.origin;
    const safeNext = next.startsWith('/') ? next : '/dashboard';
    const { error } = await locals.supabase.auth.signInWithOtp({
      email,
      options: {
        emailRedirectTo: `${siteUrl}/auth/callback?next=${encodeURIComponent(safeNext)}`
      }
    });
    if (error) return fail(400, { email, error: error.message });
    return { success: true, email, next: safeNext };
  }
};
