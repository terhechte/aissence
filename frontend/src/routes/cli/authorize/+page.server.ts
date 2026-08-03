import { env } from '$env/dynamic/private';
import { error, fail, redirect } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';

const requestIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const browserCodePattern = /^[A-Za-z0-9_-]{32,128}$/;

function authorizationPath(requestId: string, browserCode: string) {
  return `/cli/authorize?request_id=${encodeURIComponent(requestId)}&browser_code=${encodeURIComponent(browserCode)}`;
}

function validAuthorization(requestId: string, browserCode: string) {
  return requestIdPattern.test(requestId) && browserCodePattern.test(browserCode);
}

export const load: PageServerLoad = async ({ locals, url, setHeaders }) => {
  setHeaders({
    'cache-control': 'private, no-store, max-age=0',
    'referrer-policy': 'no-referrer'
  });
  const requestId = url.searchParams.get('request_id') ?? '';
  const browserCode = url.searchParams.get('browser_code') ?? '';
  if (!validAuthorization(requestId, browserCode)) error(400, 'Invalid CLI authorization link.');

  const { claims } = await locals.safeGetClaims();
  if (!claims) {
    const next = authorizationPath(requestId, browserCode);
    redirect(303, `/auth?next=${encodeURIComponent(next)}`);
  }
  return {
    requestId,
    browserCode,
    accountLabel: claims.email ?? 'your signed-in account'
  };
};

export const actions: Actions = {
  default: async ({ request, locals, fetch }) => {
    const form = await request.formData();
    const requestId = String(form.get('request_id') ?? '');
    const browserCode = String(form.get('browser_code') ?? '');
    if (!validAuthorization(requestId, browserCode)) {
      return fail(400, { error: 'Invalid CLI authorization request.' });
    }

    const { claims } = await locals.safeGetClaims();
    if (!claims) {
      const next = authorizationPath(requestId, browserCode);
      redirect(303, `/auth?next=${encodeURIComponent(next)}`);
    }
    if (!locals.supabase) return fail(503, { error: 'Supabase is not configured.' });

    const { data, error: sessionError } = await locals.supabase.auth.getSession();
    const accessToken = data.session?.access_token;
    if (sessionError || !accessToken) {
      const next = authorizationPath(requestId, browserCode);
      redirect(303, `/auth?next=${encodeURIComponent(next)}`);
    }

    const apiUrl = (env.AISSENCE_API_URL ?? 'http://127.0.0.1:8787').replace(/\/$/, '');
    const response = await fetch(
      `${apiUrl}/v1/auth/device/${encodeURIComponent(requestId)}/approve`,
      {
        method: 'POST',
        headers: {
          authorization: `Bearer ${accessToken}`,
          'content-type': 'application/json'
        },
        body: JSON.stringify({ browser_code: browserCode })
      }
    );
    const result = (await response.json().catch(() => ({}))) as {
      detail?: string;
      error?: string;
    };
    if (!response.ok) {
      return fail(400, {
        error:
          result.detail ??
          (result.error === 'unauthorized'
            ? 'This CLI authorization request has expired or was already used.'
            : result.error) ??
          'The CLI could not be authorized.'
      });
    }
    return { authorized: true };
  }
};
