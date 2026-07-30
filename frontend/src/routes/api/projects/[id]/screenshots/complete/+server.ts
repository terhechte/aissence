import { json } from '@sveltejs/kit';
import { screenshotMetadataSchema } from '$lib/aissence/project';
import { requireProjectOwner } from '$lib/server/auth';

export const POST = async (event) => {
  const claims = await requireProjectOwner(event, event.params.id);
  const body = await event.request.json().catch(() => null);
  const items: unknown[] = Array.isArray(body?.screenshots) ? body.screenshots : [];
  const parsed = items.map((item: unknown) => screenshotMetadataSchema.safeParse(item));
  const invalid = parsed.find((item) => !item.success);
  if (invalid && !invalid.success) {
    return json({ error: invalid.error.issues[0]?.message ?? 'Invalid screenshot details.' }, { status: 400 });
  }

  const { data: existing } = await event.locals.supabase!
    .from('project_screenshots')
    .select('sort_order')
    .eq('project_id', event.params.id);
  const used = new Set((existing ?? []).map((row) => row.sort_order));
  const freeOrders = Array.from({ length: 8 }, (_, index) => index).filter((index) => !used.has(index));

  const rows = parsed.map((item, index) => {
    const data = item.data!;
    if (!data.storage_path.startsWith(`${claims.sub}/${event.params.id}/`)) {
      return null;
    }
    return { project_id: event.params.id, ...data, sort_order: freeOrders[index] };
  });
  if (rows.some((row) => row === null || row.sort_order === undefined)) {
    return json({ error: 'Invalid screenshot path.' }, { status: 400 });
  }
  const validRows = rows.filter((row): row is NonNullable<typeof row> => row !== null);

  const storage = event.locals.supabase!.storage.from('project-media');
  const uploadedPaths = validRows.map((row) => row.storage_path);
  for (const row of validRows) {
    const { data: blob, error: downloadError } = await storage.download(row.storage_path);
    if (downloadError || !blob || blob.size <= 0 || blob.size > 8 * 1024 * 1024) {
      await storage.remove(uploadedPaths);
      return json({ error: 'A screenshot upload is missing or too large.' }, { status: 400 });
    }
    const bytes = new Uint8Array(await blob.slice(0, 16).arrayBuffer());
    const jpeg = bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff;
    const png =
      bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47;
    const webp =
      String.fromCharCode(...bytes.slice(0, 4)) === 'RIFF' &&
      String.fromCharCode(...bytes.slice(8, 12)) === 'WEBP';
    if (!jpeg && !png && !webp) {
      await storage.remove(uploadedPaths);
      return json({ error: 'Screenshots must contain valid JPEG, PNG, or WebP image data.' }, { status: 400 });
    }
  }

  const { error } = await event.locals.supabase!.from('project_screenshots').insert(validRows);
  if (error) return json({ error: error.message }, { status: 400 });
  return json({ ok: true });
};
