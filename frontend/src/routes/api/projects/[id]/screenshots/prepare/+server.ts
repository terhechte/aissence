import { json } from '@sveltejs/kit';
import { requireProjectOwner } from '$lib/server/auth';

const allowedTypes: Record<string, string> = {
  'image/jpeg': 'jpg',
  'image/png': 'png',
  'image/webp': 'webp'
};

export const POST = async (event) => {
  const claims = await requireProjectOwner(event, event.params.id);
  const body = await event.request.json().catch(() => null);
  const files = Array.isArray(body?.files) ? body.files : [];
  const { count } = await event.locals.supabase!
    .from('project_screenshots')
    .select('*', { count: 'exact', head: true })
    .eq('project_id', event.params.id);

  if (!files.length || files.length + (count ?? 0) > 8) {
    return json({ error: 'A project may have between 1 and 8 screenshots.' }, { status: 400 });
  }

  const uploads = [];
  for (const file of files) {
    const extension = allowedTypes[String(file.type)];
    const size = Number(file.size);
    if (!extension || !Number.isFinite(size) || size <= 0 || size > 8 * 1024 * 1024) {
      return json({ error: 'Screenshots must be JPEG, PNG, or WebP files up to 8 MB.' }, { status: 400 });
    }
    const path = `${claims.sub}/${event.params.id}/${crypto.randomUUID()}.${extension}`;
    const { data, error } = await event.locals.supabase!.storage
      .from('project-media')
      .createSignedUploadUrl(path);
    if (error) return json({ error: error.message }, { status: 400 });
    uploads.push({ path, token: data.token });
  }
  return json({ uploads });
};
