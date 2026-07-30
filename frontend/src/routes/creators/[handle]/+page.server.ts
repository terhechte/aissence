import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params }) => {
  if (!locals.supabase) error(503, 'Supabase is not configured');
  const { data: profile } = await locals.supabase
    .from('profiles')
    .select('id,handle,display_name,bio,created_at')
    .eq('handle', params.handle)
    .maybeSingle();
  if (!profile) error(404, 'Creator not found');
  const { data: projects } = await locals.supabase
    .from('projects')
    .select('id,slug,name,tagline,description,repository_label,published_at,commits(count)')
    .eq('owner_id', profile.id)
    .eq('visibility', 'published')
    .order('published_at', { ascending: false });
  return { profile, projects: projects ?? [] };
};
