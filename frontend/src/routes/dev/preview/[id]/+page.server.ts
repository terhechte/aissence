import { dev } from '$app/environment';
import { error } from '@sveltejs/kit';
import { getDevPreview } from '$lib/server/dev-preview';
import type { PageServerLoad } from './$types';

function isLoopback(hostname: string) {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]' || hostname === '::1';
}

export const load: PageServerLoad = ({ params, url, setHeaders }) => {
  if (!dev || !isLoopback(url.hostname)) error(404, 'Not found');
  const preview = getDevPreview(params.id);
  if (!preview) error(404, 'This local preview expired or the development server restarted.');
  setHeaders({ 'cache-control': 'no-store' });
  return preview;
};
