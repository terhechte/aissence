export type SubscriptionAccessRecord = {
  status: string;
  current_period_end?: string | null;
  cancel_at_period_end?: boolean | null;
};

/** Mirrors the database entitlement rule for server-rendered status and UI labels. */
export function subscriptionGrantsAccess(
  subscription: SubscriptionAccessRecord | null | undefined,
  now = Date.now()
) {
  if (!subscription) return false;
  const periodEnd = subscription.current_period_end
    ? Date.parse(subscription.current_period_end)
    : null;
  const periodIsOpen = periodEnd === null || periodEnd > now;
  if (['active', 'trialing'].includes(subscription.status)) return periodIsOpen;
  return (
    subscription.status === 'canceled' &&
    subscription.cancel_at_period_end === true &&
    periodEnd !== null &&
    periodEnd > now
  );
}

export async function hasCreatorAccess(
  supabase: any,
  userId: string | null | undefined,
  creatorId: string
) {
  if (!userId) return false;
  if (userId === creatorId) return true;
  const { data, error } = await supabase.rpc('has_creator_access', {
    p_creator_id: creatorId
  });
  return !error && data === true;
}

export function planPriceLabel(plan: { price_cents: number; currency: string; interval: string }) {
  const price = new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: plan.currency,
    minimumFractionDigits: 0,
    maximumFractionDigits: 2
  }).format(Number(plan.price_cents) / 100);
  return `${price}/${plan.interval === 'annual' ? 'year' : 'month'}`;
}
