import { fail, redirect } from '@sveltejs/kit';
import { projectMetadataSchema } from '$lib/aissence/project';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const [
    { data: project },
    { data: screenshots },
    { data: imports },
    { count: commitCount },
    { data: summaryJobs },
    { data: summaryCommits }
  ] =
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
      supabase.from('commits').select('*', { count: 'exact', head: true }).eq('project_id', params.id),
      supabase
        .from('summary_jobs')
        .select('id,kind,status,error_detail,created_at,commits!inner(project_id)')
        .eq('commits.project_id', params.id)
        .order('created_at', { ascending: false }),
      supabase
        .from('commits')
        .select('id,prompt_count,summary_status,summary_origin,public_story,summarizer_version')
        .eq('project_id', params.id)
    ]);
  if (!project) redirect(303, '/dashboard');

  let signed: Array<{ signedUrl: string | null }> = [];
  if (screenshots?.length) {
    const result = await supabase.storage
      .from('project-media')
      .createSignedUrls(screenshots.map((shot) => shot.storage_path), 60 * 60);
    signed = result.data ?? [];
  }

  const promptedCommits = ((summaryCommits ?? []) as any[]).filter(
    (commit) => Number(commit.prompt_count ?? 0) > 0
  );
  const readyCommits = promptedCommits.filter(
    (commit) =>
      commit.summary_status === 'succeeded' &&
      Number(commit.summarizer_version ?? 0) >= 5 &&
      String(commit.public_story ?? '').trim()
  );

  return {
    project: {
      ...project,
      profiles: Array.isArray(project.profiles) ? project.profiles[0] : project.profiles
    },
    imports: imports ?? [],
    summaryJobs: summaryJobs ?? [],
    summaryReadiness: {
      prompted: promptedCommits.length,
      ready: readyCommits.length,
      pending: promptedCommits.filter((commit) =>
        ['queued', 'processing'].includes(commit.summary_status)
      ).length,
      failed: promptedCommits.filter((commit) => commit.summary_status === 'failed').length,
      fallbacks: readyCommits.filter((commit) => commit.summary_origin !== 'model').length
    },
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
  prepareSummaries: async ({ locals, params }) => {
    const { data, error } = await locals.supabase!.rpc('enqueue_public_summary_jobs', {
      p_project_id: params.id
    });
    if (error) return fail(400, { error: error.message });
    return { summariesQueued: Number(data ?? 0) };
  },
  publish: async ({ locals, params }) => {
    const { error } = await locals.supabase!.rpc('publish_ready_project', {
      p_project_id: params.id
    });
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
  retrySummaries: async ({ locals, params }) => {
    const { error } = await locals.supabase!.rpc('enqueue_public_summary_jobs', {
      p_project_id: params.id
    });
    if (error) return fail(400, { error: error.message });
    return { summariesRetried: true };
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
