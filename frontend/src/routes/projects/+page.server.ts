import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
  if (!locals.supabase) return { projects: [], configured: false };

  const { data } = await locals.supabase
    .from('projects')
    .select(
      'id,slug,name,tagline,description,product_url,repository_label,repository_url,published_at,profiles!projects_owner_id_fkey(handle,display_name),commits(count)'
    )
    .eq('visibility', 'published')
    .order('published_at', { ascending: false })
    .limit(100);

  return {
    projects: (data ?? []).map((project) => ({
      ...project,
      profiles: Array.isArray(project.profiles) ? project.profiles[0] : project.profiles
    })),
    configured: true
  };
};
