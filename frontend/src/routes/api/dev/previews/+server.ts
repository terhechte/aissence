import { dev } from '$app/environment';
import { error } from '@sveltejs/kit';
import { AissenceValidationError } from '$lib/aissence/schema';
import { saveDevPreview } from '$lib/server/dev-preview';
import type { RequestHandler } from './$types';

const MAX_PREVIEW_BYTES = 32 * 1024 * 1024;

function isLoopback(hostname: string) {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]' || hostname === '::1';
}

export const GET: RequestHandler = ({ url }) => {
  if (!dev || !isLoopback(url.hostname)) error(404, 'Not found');
  return new Response(null, { status: 204 });
};

export const POST: RequestHandler = async ({ request, url }) => {
  if (!dev || !isLoopback(url.hostname)) error(404, 'Not found');

  const declaredSize = Number(request.headers.get('content-length') ?? 0);
  if (declaredSize > MAX_PREVIEW_BYTES) error(413, 'The .aissence file is too large.');

  const input = await request.text();
  if (new TextEncoder().encode(input).byteLength > MAX_PREVIEW_BYTES) {
    error(413, 'The .aissence file is too large.');
  }

  try {
    const id = saveDevPreview(input);
    return new Response(`${url.origin}/dev/preview/${id}\n`, {
      headers: { 'content-type': 'text/plain; charset=utf-8' }
    });
  } catch (caught) {
    if (caught instanceof AissenceValidationError) {
      error(400, caught.issues.join('\n'));
    }
    throw caught;
  }
};
