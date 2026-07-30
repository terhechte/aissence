import { fail } from '@sveltejs/kit';
import { saveProfile } from '$lib/server/profile';
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
  return {
    profile: profile
      ? {
          ...profile,
          avatar_url: profile.avatar_path
            ? supabase.storage.from('profile-images').getPublicUrl(profile.avatar_path).data.publicUrl
            : null
        }
      : null,
    projects: projects ?? []
  };
};

export const actions: Actions = {
  saveProfile: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { profileError: 'Sign in again.' });
    const result = await saveProfile({
      form: await request.formData(),
      supabase: locals.supabase,
      userId: claims.sub
    });
    if (!result.ok) {
      return fail(400, { profileError: result.message, profileValues: result.values });
    }
    return { profileSaved: true };
  }
};
