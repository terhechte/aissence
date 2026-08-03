import { fail, redirect } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
  const { claims } = await locals.safeGetClaims();
  if (!claims) redirect(303, '/auth?next=/account');
  return { email: claims.email ?? '' };
};

export const actions: Actions = {
  password: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { error: 'Sign in again.' });

    const form = await request.formData();
    const password = String(form.get('password') ?? '');
    const confirmation = String(form.get('password_confirmation') ?? '');

    if (password.length < 8) {
      return fail(400, { error: 'Use at least 8 characters.' });
    }
    if (password !== confirmation) {
      return fail(400, { error: 'The passwords do not match.' });
    }

    const { error } = await locals.supabase.auth.updateUser({ password });
    if (error) return fail(400, { error: error.message });
    return { passwordSaved: true };
  }
};
