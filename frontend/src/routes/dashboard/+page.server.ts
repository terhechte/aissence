import { fail } from '@sveltejs/kit';
import { profileSchema } from '$lib/aissence/project';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const [{ data: profile }, { data: projects }] = await Promise.all([
    supabase.from('profiles').select('*').eq('id', claims.sub).maybeSingle(),
    supabase
      .from('projects')
      .select('id,slug,name,tagline,visibility,published_at,updated_at,imports(status,commit_count,created_at)')
      .eq('owner_id', claims.sub)
      .order('updated_at', { ascending: false })
  ]);
  return { profile, projects: projects ?? [] };
};

export const actions: Actions = {
  saveProfile: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { profileError: 'Sign in again.' });
    const data = Object.fromEntries(await request.formData());
    const parsed = profileSchema.safeParse(data);
    if (!parsed.success) {
      return fail(400, {
        profileError: parsed.error.issues[0]?.message ?? 'Check your profile details.',
        profileValues: data
      });
    }
    const { error } = await locals.supabase.from('profiles').upsert({
      id: claims.sub,
      ...parsed.data
    });
    if (error) {
      const message = error.code === '23505' ? 'That creator handle is already taken.' : error.message;
      return fail(400, { profileError: message, profileValues: data });
    }
    return { profileSaved: true };
  }
};
