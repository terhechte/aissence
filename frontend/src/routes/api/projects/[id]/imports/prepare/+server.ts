import { json } from '@sveltejs/kit';
import { requireProjectOwner } from '$lib/server/auth';

export const POST = async (event) => {
  const claims = await requireProjectOwner(event, event.params.id);
  const body = await event.request.json().catch(() => ({}));
  const size = Number(body.size ?? 0);
  if (!Number.isFinite(size) || size <= 0 || size > 25 * 1024 * 1024) {
    return json({ error: 'The project file must be no larger than 25 MB.' }, { status: 400 });
  }

  const storage = event.locals.supabase!.storage.from('aissence-staging');
  const folder = `${claims.sub}/${event.params.id}`;
  const { data: abandoned } = await storage.list(folder, { limit: 100 });
  const stale = (abandoned ?? [])
    .filter((item) => item.updated_at && Date.now() - Date.parse(item.updated_at) > 60 * 60 * 1000)
    .map((item) => `${folder}/${item.name}`);
  if (stale.length) await storage.remove(stale);

  const path = `${claims.sub}/${event.params.id}/${crypto.randomUUID()}.aissence`;
  const { data, error } = await storage.createSignedUploadUrl(path);
  if (error) return json({ error: error.message }, { status: 400 });
  return json({ path, token: data.token });
};
