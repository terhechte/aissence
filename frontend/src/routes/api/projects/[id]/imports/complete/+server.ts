import { env } from '$env/dynamic/private';
import { json } from '@sveltejs/kit';
import { parseAissence, AissenceValidationError } from '$lib/aissence/schema';
import { requireProjectOwner } from '$lib/server/auth';

export const POST = async (event) => {
  const claims = await requireProjectOwner(event, event.params.id);
  const body = await event.request.json().catch(() => null);
  const path = typeof body?.path === 'string' ? body.path : '';
  const expectedPrefix = `${claims.sub}/${event.params.id}/`;
  if (!path.startsWith(expectedPrefix)) return json({ error: 'Invalid upload path.' }, { status: 400 });

  const storage = event.locals.supabase!.storage.from('aissence-staging');
  try {
    const { data: blob, error: downloadError } = await storage.download(path);
    if (downloadError || !blob) throw new Error(downloadError?.message ?? 'Uploaded file not found.');
    if (blob.size > 25 * 1024 * 1024) throw new Error('The project file is larger than 25 MB.');

    const raw = await blob.text();
    const parsed = parseAissence(raw);
    const { data: sessionData } = await event.locals.supabase!.auth.getSession();
    const accessToken = sessionData.session?.access_token;
    if (!accessToken) return json({ error: 'Authentication expired.' }, { status: 401 });

    const apiUrl = (env.AISSENCE_API_URL ?? 'http://127.0.0.1:8787').replace(/\/$/, '');
    const response = await event.fetch(
      `${apiUrl}/v1/projects/${encodeURIComponent(event.params.id)}/browser-imports`,
      {
        method: 'POST',
        headers: {
          authorization: `Bearer ${accessToken}`,
          'content-type': 'application/vnd.aissence+json'
        },
        body: raw
      }
    );
    const result = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(result.detail ?? result.error ?? 'The snapshot could not be applied.');

    return json({
      ok: true,
      noop: result.status === 'noop',
      commitCount: result.submitted_commit_count ?? parsed.snapshot.commits.length,
      queuedCommitCount: result.queued_commit_count ?? 0,
      warnings: parsed.warnings
    });
  } catch (error) {
    if (error instanceof AissenceValidationError) {
      return json({ error: error.message, issues: error.issues }, { status: 400 });
    }
    return json({ error: error instanceof Error ? error.message : 'Import failed.' }, { status: 400 });
  } finally {
    await storage.remove([path]);
  }
};
