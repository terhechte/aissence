import { fail, redirect } from '@sveltejs/kit';
import { saveProfile } from '$lib/server/profile';
import type { Actions, PageServerLoad } from './$types';

function parsePriceCents(value: FormDataEntryValue | null) {
  const raw = String(value ?? '').trim();
  if (!raw) return null;
  const dollars = Number(raw);
  if (!Number.isFinite(dollars) || dollars <= 0 || dollars > 1_000_000) return undefined;
  const cents = Math.round(dollars * 100);
  return cents > 0 ? cents : undefined;
}

export const load: PageServerLoad = async ({ locals, parent }) => {
  const { claims } = await parent();
  const supabase = locals.supabase!;
  const [{ data: profile }, { data: plans }, { data: account }] = await Promise.all([
    supabase
      .from('profiles')
      .select('id,handle,display_name,bio,avatar_path')
      .eq('id', claims.sub)
      .maybeSingle(),
    supabase
      .from('creator_plans')
      .select('id,name,description,interval,price_cents,currency,active')
      .eq('creator_id', claims.sub)
      .order('interval'),
    supabase
      .from('creator_accounts')
      .select('status,kyc_status,payout_status,provider,updated_at')
      .eq('creator_id', claims.sub)
      .maybeSingle()
  ]);

  if (!profile) redirect(303, '/dashboard');

  return {
    profile: {
      ...profile,
      avatar_url: profile.avatar_path
        ? supabase.storage.from('profile-images').getPublicUrl(profile.avatar_path).data.publicUrl
        : null
    },
    plans: plans ?? [],
    account: account ?? null
  };
};

export const actions: Actions = {
  saveProfile: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { error: 'Sign in again.' });

    const { data: current } = await locals.supabase
      .from('profiles')
      .select('avatar_path')
      .eq('id', claims.sub)
      .maybeSingle();
    if (!current) return fail(404, { error: 'Creator profile not found.' });

    const result = await saveProfile({
      form: await request.formData(),
      supabase: locals.supabase,
      userId: claims.sub,
      currentAvatarPath: current.avatar_path
    });
    if (!result.ok) {
      return fail(400, { error: result.message, profileValues: result.values });
    }
    return { saved: true };
  },
  savePlans: async ({ request, locals }) => {
    const { claims } = await locals.safeGetClaims();
    if (!claims || !locals.supabase) return fail(401, { planError: 'Sign in again.' });

    const form = await request.formData();
    const monthlyCents = parsePriceCents(form.get('monthly_price'));
    const annualCents = parsePriceCents(form.get('annual_price'));
    if (monthlyCents === undefined || annualCents === undefined) {
      return fail(400, {
        planError: 'Enter valid positive prices, or leave a plan blank to hide it.',
        planValues: {
          monthly_price: String(form.get('monthly_price') ?? ''),
          annual_price: String(form.get('annual_price') ?? '')
        }
      });
    }

    const plans = [
      {
        interval: 'monthly',
        name: 'Monthly',
        description: 'Full retained prompts across every published ledger.',
        price_cents: monthlyCents
      },
      {
        interval: 'annual',
        name: 'Annual',
        description: 'Full retained prompts across every published ledger.',
        price_cents: annualCents
      }
    ];

    for (const plan of plans) {
      if (plan.price_cents === null) {
        const { error } = await locals.supabase
          .from('creator_plans')
          .update({ active: false })
          .eq('creator_id', claims.sub)
          .eq('interval', plan.interval);
        if (error) return fail(400, { planError: error.message });
        continue;
      }

      const { error } = await locals.supabase.from('creator_plans').upsert(
        {
          creator_id: claims.sub,
          name: plan.name,
          description: plan.description,
          interval: plan.interval,
          price_cents: plan.price_cents,
          currency: 'USD',
          active: true
        },
        { onConflict: 'creator_id,interval' }
      );
      if (error) return fail(400, { planError: error.message });
    }

    return { plansSaved: true };
  }
};
