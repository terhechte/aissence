<script lang="ts">
  import { formatDate } from '$lib/utils';

  let { data } = $props();

  function statusLabel(subscription: any) {
    if (subscription.cancel_at_period_end && subscription.current_period_end) return 'canceling';
    if (subscription.status === 'canceled') return 'expired';
    return subscription.status;
  }

  function planPrice(plan: any) {
    if (!plan) return 'Creator access';
    const amount = new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: plan.currency,
      minimumFractionDigits: 0,
      maximumFractionDigits: 2
    }).format(Number(plan.price_cents) / 100);
    return `${amount}/${plan.interval === 'annual' ? 'year' : 'month'}`;
  }
</script>

<svelte:head><title>Subscriptions — aissence</title></svelte:head>

<div class="page-shell subscriptions">
  <header>
    <div>
      <p class="eyebrow">Your reading list</p>
      <h1>Subscriptions.</h1>
      <p>Every published ledger unlocked by your active creator subscriptions.</p>
    </div>
  </header>

  <section class="billing">
    <div class="section-title">
      <div>
        <p class="label">Creator subscriptions</p>
        <p>{data.subscriptions?.length ?? 0} active or historical</p>
      </div>
      <span class="provider-note">Payment provider controls are coming with checkout.</span>
    </div>

    {#if data.subscriptions?.length}
      <div class="subscription-list">
        {#each data.subscriptions as subscription}
          <article class:inactive={!subscription.access}>
            <div>
              <span class="label">{statusLabel(subscription)}</span>
              <h2>{subscription.creator?.display_name ?? 'Creator'}</h2>
              <p>{subscription.creator ? `@${subscription.creator.handle}` : 'Creator unavailable'}</p>
            </div>
            <div>
              <strong>{planPrice(subscription.plan)}</strong>
              <small>{subscription.source === 'manual' ? 'Manual access' : 'Provider subscription'}</small>
            </div>
            <div>
              {#if subscription.current_period_end}
                <strong>{subscription.cancel_at_period_end ? 'Access until' : 'Renews'} {formatDate(subscription.current_period_end)}</strong>
              {:else}
                <strong>{subscription.access ? 'No expiry set' : 'Expired'}</strong>
              {/if}
              {#if subscription.cancel_at_period_end}<small>Cancellation takes effect at period end.</small>{/if}
            </div>
          </article>
        {/each}
      </div>
    {:else}
      <p class="billing-empty">Subscribe to a creator to build your reading list.</p>
    {/if}
  </section>

  <section>
    <div class="section-title">
      <div>
        <p class="label">Subscribed ledgers</p>
        <p>{data.ledgers.length} {data.ledgers.length === 1 ? 'ledger' : 'ledgers'}</p>
      </div>
    </div>

    {#if data.ledgers.length}
      <div class="ledger-list">
        {#each data.ledgers as ledger}
          <a href={`/creators/${ledger.creator.handle}/${ledger.slug}`}>
            <div>
              <h2>{ledger.name}</h2>
              {#if ledger.tagline}<p>{ledger.tagline}</p>{/if}
            </div>
            <div class="creator">
              <span>Creator</span>
              <strong>{ledger.creator.display_name}</strong>
              <small>@{ledger.creator.handle}</small>
            </div>
            <time>Updated {formatDate(ledger.updated_at)}</time>
          </a>
        {/each}
      </div>
    {:else}
      <div class="empty-state">
        <p>You are not subscribed to any ledgers yet.</p>
        <a href="/#recent">Browse public ledgers →</a>
      </div>
    {/if}
  </section>
</div>

<style>
  .subscriptions {
    padding-block: 55px 120px;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: 30px;
    margin-bottom: 70px;
  }
  header h1 {
    margin: 16px 0;
    font: 400 clamp(62px, 9vw, 104px) / 0.9 'Instrument Serif', Georgia, serif;
  }
  header p:last-child {
    max-width: 600px;
    color: var(--muted);
    font-size: 20px;
  }
  section {
    border-top: 1px solid var(--ink);
    padding-top: 22px;
  }
  .billing {
    margin-bottom: 65px;
  }
  .section-title {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    border-bottom: 1px solid var(--ink);
    padding-bottom: 15px;
  }
  .section-title p {
    margin: 0 0 6px;
    color: var(--muted);
  }
  .provider-note,
  .billing-empty {
    color: var(--muted);
    font-size: 13px;
  }
  .subscription-list {
    border-bottom: 1px solid var(--rule);
  }
  .subscription-list article {
    display: grid;
    grid-template-columns: minmax(0, 1.3fr) minmax(150px, 0.7fr) minmax(190px, 0.9fr);
    align-items: center;
    gap: 24px;
    border-bottom: 1px solid var(--rule);
    padding: 18px 8px 18px 0;
  }
  .subscription-list article:last-child {
    border-bottom: 0;
  }
  .subscription-list article.inactive {
    opacity: 0.62;
  }
  .subscription-list h2 {
    margin: 8px 0 3px;
    font: 400 27px/1.15 'Instrument Serif', Georgia, serif;
  }
  .subscription-list p,
  .subscription-list small {
    color: var(--muted);
  }
  .subscription-list p {
    margin: 0;
  }
  .subscription-list article > div:not(:first-child) {
    display: grid;
    gap: 5px;
  }
  .ledger-list a {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 220px 145px;
    align-items: center;
    gap: 25px;
    border-bottom: 1px solid var(--rule);
    padding: 20px 8px 20px 0;
    text-decoration: none;
  }
  .ledger-list a:hover {
    background: var(--panel2);
  }
  .ledger-list h2 {
    margin: 0;
    font: 400 27px/1.15 'Instrument Serif', Georgia, serif;
  }
  .ledger-list p {
    margin: 5px 0 0;
    color: var(--soft);
  }
  .creator {
    display: grid;
    gap: 4px;
  }
  .creator span,
  .creator small,
  .ledger-list time {
    color: var(--muted);
    font: 400 10px/1.3 'JetBrains Mono', monospace;
  }
  .creator span {
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  .creator strong {
    font-weight: 400;
  }
  @media (max-width: 720px) {
    header {
      align-items: flex-start;
      flex-direction: column;
    }
    .ledger-list a {
      grid-template-columns: 1fr auto;
    }
    .section-title {
      align-items: flex-start;
      flex-direction: column;
      gap: 12px;
    }
    .subscription-list article {
      grid-template-columns: 1fr 1fr;
    }
    .subscription-list article > div:first-child {
      grid-column: 1 / -1;
    }
    .ledger-list time {
      display: none;
    }
  }
</style>
