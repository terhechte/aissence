import { createBrowserClient } from '@supabase/ssr';
import { publicSupabaseConfig } from './config';

let client: ReturnType<typeof createBrowserClient> | null = null;

export function getBrowserSupabase() {
  const config = publicSupabaseConfig();
  if (!config) return null;
  client ??= createBrowserClient(config.url, config.key);
  return client;
}
