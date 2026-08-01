import { describe, expect, it } from 'vitest';
import { subscriptionGrantsAccess } from './subscriptions';

const now = Date.parse('2026-08-01T00:00:00Z');

describe('subscriptionGrantsAccess', () => {
  it('keeps an active subscription open until its period ends', () => {
    expect(
      subscriptionGrantsAccess(
        { status: 'active', current_period_end: '2026-09-01T00:00:00Z' },
        now
      )
    ).toBe(true);
  });

  it('keeps a canceled-at-period-end subscription open until the paid period ends', () => {
    expect(
      subscriptionGrantsAccess(
        {
          status: 'canceled',
          cancel_at_period_end: true,
          current_period_end: '2026-08-02T00:00:00Z'
        },
        now
      )
    ).toBe(true);
  });

  it('does not grant access for an immediately canceled or past-due subscription', () => {
    expect(
      subscriptionGrantsAccess(
        {
          status: 'canceled',
          cancel_at_period_end: false,
          current_period_end: '2026-09-01T00:00:00Z'
        },
        now
      )
    ).toBe(false);
    expect(subscriptionGrantsAccess({ status: 'past_due' }, now)).toBe(false);
  });
});
