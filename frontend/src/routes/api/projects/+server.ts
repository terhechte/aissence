import { json } from '@sveltejs/kit';
import { projectMetadataSchema } from '$lib/aissence/project';
import { requireApiClaims } from '$lib/server/auth';

export const POST = async (event) => {
  const claims = await requireApiClaims(event);
  const body = await event.request.json().catch(() => null);
  const parsed = projectMetadataSchema.safeParse(body);
  if (!parsed.success) {
    return json({ error: parsed.error.issues[0]?.message ?? 'Invalid project details' }, { status: 400 });
  }

  const { data: profile } = await event.locals.supabase!
    .from('profiles')
    .select('id')
    .eq('id', claims.sub)
    .maybeSingle();
  if (!profile) return json({ error: 'Create your creator profile first.' }, { status: 409 });

  const { data, error } = await event.locals.supabase!.rpc('create_owned_project', {
    p_name: parsed.data.name,
    p_slug: parsed.data.slug,
    p_tagline: parsed.data.tagline,
    p_description: parsed.data.description,
    p_product_url: parsed.data.product_url,
    p_repository_label: parsed.data.repository_label ?? '',
    p_repository_url: parsed.data.repository_url ?? ''
  });
  if (error) {
    return json(
      { error: error.code === '23505' ? 'That project slug is already in use.' : error.message },
      { status: 400 }
    );
  }
  return json({ id: data }, { status: 201 });
};
