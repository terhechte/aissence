import { fail, redirect } from '@sveltejs/kit';
import { projectMetadataSchema } from '$lib/aissence/project';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const [{ data: project }, { data: screenshots }, { data: imports }, { count: commitCount }] =
    await Promise.all([
      supabase
        .from('projects')
        .select('*,profiles!projects_owner_id_fkey(handle,display_name)')
        .eq('id', params.id)
        .eq('owner_id', claims.sub)
        .maybeSingle(),
      supabase
        .from('project_screenshots')
        .select('*')
        .eq('project_id', params.id)
        .order('sort_order'),
      supabase
        .from('imports')
        .select('*')
        .eq('project_id', params.id)
        .order('created_at', { ascending: false })
        .limit(10),
      supabase.from('commits').select('*', { count: 'exact', head: true }).eq('project_id', params.id)
    ]);
  if (!project) redirect(303, '/dashboard');

  let signed: Array<{ signedUrl: string | null }> = [];
  if (screenshots?.length) {
    const result = await supabase.storage
      .from('project-media')
      .createSignedUrls(screenshots.map((shot) => shot.storage_path), 60 * 60);
    signed = result.data ?? [];
  }

  return {
    project: {
      ...project,
      profiles: Array.isArray(project.profiles) ? project.profiles[0] : project.profiles
    },
    imports: imports ?? [],
    commitCount: commitCount ?? 0,
    screenshots: (screenshots ?? []).map((shot, index) => ({
      ...shot,
      url: signed[index]?.signedUrl ?? ''
    }))
  };
};

export const actions: Actions = {
  updateMetadata: async ({ request, locals, params }) => {
    const parsed = projectMetadataSchema.safeParse(Object.fromEntries(await request.formData()));
    if (!parsed.success) {
      return fail(400, { error: parsed.error.issues[0]?.message ?? 'Check the project details.' });
    }
    const { error } = await locals.supabase!
      .from('projects')
      .update(parsed.data)
      .eq('id', params.id);
    if (error) {
      return fail(400, {
        error: error.code === '23505' ? 'That project slug is already in use.' : error.message
      });
    }
    return { saved: true };
  },
  publish: async ({ locals, params }) => {
    const [{ count: screenshots }, { data: imported }] = await Promise.all([
      locals.supabase!
        .from('project_screenshots')
        .select('*', { count: 'exact', head: true })
        .eq('project_id', params.id),
      locals.supabase!
        .from('imports')
        .select('id')
        .eq('project_id', params.id)
        .in('status', ['succeeded', 'noop'])
        .limit(1)
        .maybeSingle()
    ]);
    if (!screenshots || !imported) {
      return fail(400, { error: 'A successful import and at least one screenshot are required.' });
    }
    const { error } = await locals.supabase!
      .from('projects')
      .update({ visibility: 'published', published_at: new Date().toISOString() })
      .eq('id', params.id);
    if (error) return fail(400, { error: error.message });
    return { published: true };
  },
  unpublish: async ({ locals, params }) => {
    const { error } = await locals.supabase!
      .from('projects')
      .update({ visibility: 'draft', published_at: null })
      .eq('id', params.id);
    if (error) return fail(400, { error: error.message });
    return { unpublished: true };
  },
  moveScreenshot: async ({ request, locals }) => {
    const form = await request.formData();
    const id = String(form.get('screenshot_id') ?? '');
    const direction = Number(form.get('direction') ?? 0);
    const { error } = await locals.supabase!.rpc('move_project_screenshot', {
      p_screenshot_id: id,
      p_direction: direction
    });
    if (error) return fail(400, { error: error.message });
    return { moved: true };
  },
  removeScreenshot: async ({ request, locals, params }) => {
    const id = String((await request.formData()).get('screenshot_id') ?? '');
    const { data: shot } = await locals.supabase!
      .from('project_screenshots')
      .select('storage_path')
      .eq('id', id)
      .eq('project_id', params.id)
      .maybeSingle();
    if (!shot) return fail(404, { error: 'Screenshot not found.' });
    const { error } = await locals.supabase!.from('project_screenshots').delete().eq('id', id);
    if (error) return fail(400, { error: error.message });
    await locals.supabase!.storage.from('project-media').remove([shot.storage_path]);
    const { data: remaining } = await locals.supabase!
      .from('project_screenshots')
      .select('id,sort_order')
      .eq('project_id', params.id)
      .order('sort_order');
    for (let index = 0; index < (remaining ?? []).length; index += 1) {
      if (remaining![index].sort_order !== index) {
        await locals.supabase!
          .from('project_screenshots')
          .update({ sort_order: index })
          .eq('id', remaining![index].id);
      }
    }
    return { removed: true };
  },
  deleteProject: async ({ locals, params }) => {
    const { data: shots } = await locals.supabase!
      .from('project_screenshots')
      .select('storage_path')
      .eq('project_id', params.id);
    if (shots?.length) {
      await locals.supabase!.storage
        .from('project-media')
        .remove(shots.map((shot) => shot.storage_path));
    }
    const { error } = await locals.supabase!.from('projects').delete().eq('id', params.id);
    if (error) return fail(400, { error: error.message });
    redirect(303, '/dashboard');
  }
};
