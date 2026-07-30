import { createServerClient } from '@supabase/ssr';
import type { Handle } from '@sveltejs/kit';
import { env } from '$env/dynamic/public';

export const handle: Handle = async ({ event, resolve }) => {
  const url = env.PUBLIC_SUPABASE_URL;
  const key = env.PUBLIC_SUPABASE_PUBLISHABLE_KEY;

  if (!url || !key) {
    event.locals.supabase = null;
    event.locals.safeGetClaims = async () => ({ claims: null });
    return resolve(event);
  }

  event.locals.supabase = createServerClient(url, key, {
    cookies: {
      getAll: () => event.cookies.getAll(),
      setAll: (cookiesToSet) => {
        for (const { name, value, options } of cookiesToSet) {
          event.cookies.set(name, value, { ...options, path: '/' });
        }
      }
    }
  });

  event.locals.safeGetClaims = async () => {
    try {
      const { data, error } = await event.locals.supabase!.auth.getClaims();
      if (error || !data?.claims?.sub) return { claims: null };
      return {
        claims: {
          sub: String(data.claims.sub),
          email: typeof data.claims.email === 'string' ? data.claims.email : undefined
        }
      };
    } catch {
      return { claims: null };
    }
  };

  return resolve(event, {
    filterSerializedResponseHeaders: (name) =>
      name === 'content-range' || name === 'x-supabase-api-version'
  });
};
