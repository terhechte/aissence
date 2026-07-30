import { createHash } from 'node:crypto';
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
    const checksum = createHash('sha256').update(raw).digest('hex');

    const { data: importRow, error: importError } = await event.locals.supabase!
      .from('imports')
      .insert({
        project_id: event.params.id,
        uploader_id: claims.sub,
        checksum,
        source_version: parsed.file.version,
        status: 'processing',
        warnings: parsed.warnings
      })
      .select('id')
      .single();
    if (importError) throw new Error(importError.message);

    const { data: result, error: rpcError } = await event.locals.supabase!.rpc(
      'replace_project_snapshot',
      {
        p_project_id: event.params.id,
        p_import_id: importRow.id,
        p_checksum: checksum,
        p_source_version: parsed.file.version,
        p_snapshot: parsed.snapshot
      }
    );
    if (rpcError) throw new Error(rpcError.message);
    if (!result?.ok) throw new Error(result?.error ?? 'The snapshot could not be applied.');

    return json({
      ok: true,
      noop: Boolean(result.noop),
      commitCount: parsed.snapshot.commits.length,
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
