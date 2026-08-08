/**
 * Attach the newest usable commit summary to project cards without another
 * model call. The detailed ledger builds a broader milestone arc client-side;
 * directory surfaces only need one compact, cached overview.
 */
export async function attachProjectOverviews(supabase: any, projects: any[]) {
  const ids = projects.map((project) => project.id).filter(Boolean);
  if (!ids.length) return projects;

  const { data } = await supabase
    .from('project_summary_overviews')
    .select('project_id,title,public_subline,public_story,sort_order')
    .in('project_id', ids)
    .order('sort_order', { ascending: false });

  const newest = new Map<string, any>();
  for (const commit of (data ?? []) as any[]) {
    if (!newest.has(commit.project_id)) newest.set(commit.project_id, commit);
  }

  return projects.map((project) => {
    const commit = newest.get(project.id);
    return {
      ...project,
      summary_overview: String(commit?.public_subline || commit?.public_story || '').trim() || null,
      summary_commit_title: commit?.title ?? null
    };
  });
}
