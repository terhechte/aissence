<script lang="ts">
  import { formatDate } from '$lib/utils';

  let { data } = $props();
</script>

<svelte:head><title>Subscriptions — aissence</title></svelte:head>

<div class="page-shell subscriptions">
  <header>
    <div>
      <p class="eyebrow">Your reading list</p>
      <h1>Subscriptions.</h1>
      <p>Every published ledger unlocked by your active creator subscriptions.</p>
    </div>
    <a class="button secondary" href="/dashboard">Back to dashboard</a>
  </header>

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
              <p>{ledger.tagline}</p>
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
    .ledger-list time {
      display: none;
    }
  }
</style>
