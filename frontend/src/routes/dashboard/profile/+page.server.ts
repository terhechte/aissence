import { fail, redirect } from '@sveltejs/kit';
import { saveProfile } from '$lib/server/profile';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const { data: profile } = await supabase
    .from('profiles')
    .select('id,handle,display_name,bio,avatar_path')
    .eq('id', claims.sub)
    .maybeSingle();

  if (!profile) redirect(303, '/dashboard');

  return {
    profile: {
      ...profile,
      avatar_url: profile.avatar_path
        ? supabase.storage.from('profile-images').getPublicUrl(profile.avatar_path).data.publicUrl
        : null
    }
  };
};

export const actions: Actions = {
  saveProfile: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { error: 'Sign in again.' });

    const { data: current } = await locals.supabase
      .from('profiles')
      .select('avatar_path')
      .eq('id', claims.sub)
      .maybeSingle();
    if (!current) return fail(404, { error: 'Creator profile not found.' });

    const result = await saveProfile({
      form: await request.formData(),
      supabase: locals.supabase,
      userId: claims.sub,
      currentAvatarPath: current.avatar_path
    });
    if (!result.ok) {
      return fail(400, { error: result.message, profileValues: result.values });
    }
    return { saved: true };
  }
};
