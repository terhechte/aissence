import { profileSchema } from '$lib/aissence/project';

const avatarTypes = {
  'image/jpeg': 'jpg',
  'image/png': 'png',
  'image/webp': 'webp'
} as const;

export type ProfileFormValues = {
  display_name: string;
  handle: string;
  bio: string;
};

export type ProfileSaveResult =
  | { ok: true; avatarPath: string | null }
  | { ok: false; message: string; values: ProfileFormValues };

function readProfileForm(form: FormData): ProfileFormValues {
  return {
    display_name: String(form.get('display_name') ?? ''),
    handle: String(form.get('handle') ?? ''),
    bio: String(form.get('bio') ?? '')
  };
}

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

export async function saveProfile({
  form,
  supabase,
  userId,
  currentAvatarPath = null
}: {
  form: FormData;
  supabase: any;
  userId: string;
  currentAvatarPath?: string | null;
}): Promise<ProfileSaveResult> {
  const values = readProfileForm(form);
  const parsed = profileSchema.safeParse(values);
  if (!parsed.success) {
    return {
      ok: false,
      message: parsed.error.issues[0]?.message ?? 'Check your profile details.',
      values
    };
  }

  const avatar = form.get('avatar');
  let avatarPath: string | null = null;
  if (avatar instanceof File && avatar.size > 0) {
    if (avatar.size > 5 * 1024 * 1024) {
      return { ok: false, message: 'Profile images must be no larger than 5 MB.', values };
    }
    const extension = await validAvatarExtension(avatar);
    if (!extension) {
      return { ok: false, message: 'Choose a valid JPEG, PNG, or WebP profile image.', values };
    }
    avatarPath = `${userId}/${crypto.randomUUID()}.${extension}`;
    const { error: uploadError } = await supabase.storage
      .from('profile-images')
      .upload(avatarPath, avatar, { contentType: avatar.type, upsert: false });
    if (uploadError) {
      return { ok: false, message: `The profile image could not be uploaded: ${uploadError.message}`, values };
    }
  }

  const update = {
    id: userId,
    ...parsed.data,
    ...(avatarPath ? { avatar_path: avatarPath } : {})
  };
  const { error } = await supabase.from('profiles').upsert(update);
  if (error) {
    if (avatarPath) await supabase.storage.from('profile-images').remove([avatarPath]);
    return {
      ok: false,
      message: error.code === '23505' ? 'That creator handle is already taken.' : error.message,
      values
    };
  }

  if (avatarPath && currentAvatarPath && currentAvatarPath !== avatarPath) {
    await supabase.storage.from('profile-images').remove([currentAvatarPath]);
  }

  return { ok: true, avatarPath };
}
