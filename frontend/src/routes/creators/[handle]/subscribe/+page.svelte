<script lang="ts">
  import { formatDate } from '$lib/utils';
  let { data } = $props();

  function formatPrice(cents: number, currency: string) {
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency,
      minimumFractionDigits: 0,
      maximumFractionDigits: 2
    }).format(Number(cents) / 100);
  }
</script>

<svelte:head><title>Subscribe to {data.profile.display_name} — aissence</title></svelte:head>

<div class="page-shell subscribe">
  <section>
    <p class="eyebrow">Creator subscription</p>
    <h1>Read every prompt from {data.profile.display_name}.</h1>
    <p>
      One subscription unlocks the retained human prompts across every current and future
      published ledger. Summaries remain free for everyone.
    </p>
    {#if data.isEntitled}
      <div class="notice">
        Your access is active{data.subscription?.cancel_at_period_end && data.subscription.current_period_end
          ? ` through ${formatDate(data.subscription.current_period_end)}`
          : data.subscription?.current_period_end
            ? ` until ${formatDate(data.subscription.current_period_end)}`
            : ''}.
        {#if data.subscription?.cancel_at_period_end} Cancellation is scheduled for the end of the paid period.{/if}
      </div>
      <a class="button" href={`/creators/${data.profile.handle}`}>View projects</a>
    {:else}
      {#if data.plans.length}
        <div class="plans" aria-label="Subscription plans">
          {#each data.plans as plan}
            <article class="plan-card">
              <div>
                <span class="label">{plan.interval === 'annual' ? 'Annual' : 'Monthly'}</span>
                <h2>{formatPrice(plan.price_cents, plan.currency)}<small>/{plan.interval === 'annual' ? 'year' : 'month'}</small></h2>
                <p>{plan.description || 'Full retained prompts across every published ledger.'}</p>
              </div>
              <button class="button secondary" type="button" disabled>{data.paymentsReady ? 'Checkout pending' : 'KYC pending'}</button>
            </article>
          {/each}
        </div>
      {:else}
        <div class="pending">
          <span class="label">Plans are being prepared</span>
          <p>This creator has not published a monthly or annual subscription plan yet.</p>
        </div>
      {/if}
      <p class="provider-note">
        Payment and KYC are handled by the selected marketplace provider. Checkout will appear
        here once KYC is complete and that provider is connected.
      </p>
      <a class="button secondary" href={`/creators/${data.profile.handle}`}>Back to creator</a>
    {/if}
  </section>
</div>

<style>
  .subscribe {
    min-height: calc(100vh - 79px);
    display: grid;
    place-items: center;
    padding-block: 70px;
  }
  section {
    max-width: 820px;
    border-top: 1px solid var(--ink);
    padding-top: 25px;
  }
  h1 {
    font: 400 clamp(60px, 9vw, 100px) / 0.92 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.025em;
    margin: 20px 0 26px;
  }
  section > p {
    max-width: 620px;
    color: var(--body);
    font-size: 21px;
    line-height: 1.5;
  }
  .notice,
  .pending {
    margin: 30px 0 22px;
    padding: 22px;
  }
  .pending {
    background: var(--panel);
  }
  .pending p {
    margin-bottom: 0;
    color: var(--soft);
  }
  .plans {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 15px;
    margin: 30px 0 18px;
  }
  .plan-card {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 28px;
    min-height: 230px;
    padding: 22px;
    border: 1px solid var(--rule2);
    background: var(--panel);
  }
  .plan-card h2 {
    margin: 18px 0 10px;
    font: 400 42px/1 'Instrument Serif', Georgia, serif;
  }
  .plan-card h2 small {
    color: var(--muted);
    font: 400 13px/1 'JetBrains Mono', monospace;
  }
  .plan-card p,
  .provider-note {
    color: var(--soft);
  }
  .plan-card .button {
    align-self: flex-start;
  }
  .provider-note {
    margin: 0 0 22px;
    font-size: 14px;
  }
  button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  @media (max-width: 720px) {
    .plans {
      grid-template-columns: 1fr;
    }
  }
</style>
