import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params }) => {
  if (!locals.supabase) error(503, 'Supabase is not configured');
  const { data: profile } = await locals.supabase
    .from('profiles')
    .select('id,handle,display_name,bio,avatar_path,created_at')
    .eq('handle', params.handle)
    .maybeSingle();
  if (!profile) error(404, 'Creator not found');
  const [{ data: projects }, { data: plans }] = await Promise.all([
    locals.supabase
      .from('projects')
      .select('id,slug,name,tagline,description,repository_label,published_at,commits(count)')
      .eq('owner_id', profile.id)
      .eq('visibility', 'published')
      .order('published_at', { ascending: false }),
    locals.supabase
      .from('creator_plans')
      .select('id,name,interval,price_cents,currency')
      .eq('creator_id', profile.id)
      .eq('active', true)
      .order('interval')
  ]);
  return {
    profile: {
      ...profile,
      avatar_url: profile.avatar_path
        ? locals.supabase.storage.from('profile-images').getPublicUrl(profile.avatar_path).data.publicUrl
        : null
    },
    projects: projects ?? [],
    plans: plans ?? []
  };
};
