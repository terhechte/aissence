import type { PageServerLoad } from './$types';
import { attachProjectOverviews } from '$lib/server/project-overviews';

export const load: PageServerLoad = async ({ locals }) => {
  if (!locals.supabase) return { projects: [], configured: false };

  const { data } = await locals.supabase
    .from('projects')
    .select(
      'id,slug,name,tagline,repository_label,published_at,owner_id,profiles!projects_owner_id_fkey(handle,display_name),commits(count)'
    )
    .eq('visibility', 'published')
    .order('published_at', { ascending: false })
    .limit(6);

  const projects = (data ?? []).map((project) => ({
      ...project,
      profiles: Array.isArray(project.profiles) ? project.profiles[0] : project.profiles
    }));
  return {
    projects: await attachProjectOverviews(locals.supabase, projects),
    configured: true
  };
};
