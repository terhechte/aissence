import { redirect } from '@sveltejs/kit';

export const GET = async ({ url, locals }) => {
  const code = url.searchParams.get('code');
  const requestedNext = url.searchParams.get('next');
  const next = requestedNext?.startsWith('/') ? requestedNext : '/dashboard';
  if (code && locals.supabase) {
    const { error } = await locals.supabase.auth.exchangeCodeForSession(code);
    if (!error) redirect(303, next);
  }
  redirect(303, '/auth?error=callback');
};
