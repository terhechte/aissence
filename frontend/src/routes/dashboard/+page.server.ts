import { fail } from '@sveltejs/kit';
import { profileSchema } from '$lib/aissence/project';
import type { Actions, PageServerLoad } from './$types';

const avatarTypes = {
  'image/jpeg': 'jpg',
  'image/png': 'png',
  'image/webp': 'webp'
} as const;

async function validAvatarExtension(file: File) {
  const expected = avatarTypes[file.type as keyof typeof avatarTypes];
  if (!expected) return null;
  const bytes = new Uint8Array(await file.slice(0, 16).arrayBuffer());
  const detected =
    bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff
      ? 'jpg'
      : bytes[0] === 0x89 &&
          bytes[1] === 0x50 &&
          bytes[2] === 0x4e &&
          bytes[3] === 0x47
        ? 'png'
        : String.fromCharCode(...bytes.slice(0, 4)) === 'RIFF' &&
            String.fromCharCode(...bytes.slice(8, 12)) === 'WEBP'
          ? 'webp'
          : null;
  return detected === expected ? detected : null;
}

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const [{ data: profile }, { data: projects }] = await Promise.all([
    supabase.from('profiles').select('*').eq('id', claims.sub).maybeSingle(),
    supabase
      .from('projects')
      .select('id,slug,name,tagline,visibility,published_at,updated_at,imports(status,commit_count,created_at)')
      .eq('owner_id', claims.sub)
      .order('updated_at', { ascending: false })
  ]);
  return {
    profile: profile
      ? {
          ...profile,
          avatar_url: profile.avatar_path
            ? supabase.storage.from('profile-images').getPublicUrl(profile.avatar_path).data.publicUrl
            : null
        }
      : null,
    projects: projects ?? []
  };
};

export const actions: Actions = {
  saveProfile: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { profileError: 'Sign in again.' });
    const form = await request.formData();
    const data = {
      display_name: String(form.get('display_name') ?? ''),
      handle: String(form.get('handle') ?? ''),
      bio: String(form.get('bio') ?? '')
    };
    const parsed = profileSchema.safeParse(data);
    if (!parsed.success) {
      return fail(400, {
        profileError: parsed.error.issues[0]?.message ?? 'Check your profile details.',
        profileValues: data
      });
    }
    const avatar = form.get('avatar');
    let avatarPath: string | null = null;
    if (avatar instanceof File && avatar.size > 0) {
      if (avatar.size > 5 * 1024 * 1024) {
        return fail(400, {
          profileError: 'Profile images must be no larger than 5 MB.',
          profileValues: data
        });
      }
      const extension = await validAvatarExtension(avatar);
      if (!extension) {
        return fail(400, {
          profileError: 'Choose a valid JPEG, PNG, or WebP profile image.',
          profileValues: data
        });
      }
      avatarPath = `${claims.sub}/${crypto.randomUUID()}.${extension}`;
      const { error: uploadError } = await locals.supabase.storage
        .from('profile-images')
        .upload(avatarPath, avatar, { contentType: avatar.type, upsert: false });
      if (uploadError) {
        return fail(400, {
          profileError: `The profile image could not be uploaded: ${uploadError.message}`,
          profileValues: data
        });
      }
    }

    const { error } = await locals.supabase.from('profiles').upsert({
      id: claims.sub,
      ...parsed.data,
      avatar_path: avatarPath
    });
    if (error) {
      if (avatarPath) {
        await locals.supabase.storage.from('profile-images').remove([avatarPath]);
      }
      const message = error.code === '23505' ? 'That creator handle is already taken.' : error.message;
      return fail(400, { profileError: message, profileValues: data });
    }
    return { profileSaved: true };
  }
};
