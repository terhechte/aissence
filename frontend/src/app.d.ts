import type { SupabaseClient } from '@supabase/supabase-js';

declare global {
  namespace App {
    interface Locals {
      supabase: SupabaseClient | null;
      safeGetClaims: () => Promise<{ claims: { sub: string; email?: string } | null }>;
    }
    interface PageData {
      claims?: { sub: string; email?: string } | null;
    }
  }
}

export {};
