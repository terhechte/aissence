<script lang="ts">
  import { formatDate } from '$lib/utils';
  let { data } = $props();
</script>

<svelte:head><title>Subscribe to {data.profile.display_name} — aissence</title></svelte:head>

<div class="page-shell subscribe">
  <section>
    <p class="eyebrow">Creator subscription</p>
    <h1>Read every prompt from {data.profile.display_name}.</h1>
    <p>
      A creator subscription unlocks the retained human prompts across every published project,
      while summaries remain free for everyone.
    </p>
    {#if data.subscription && ['active', 'trialing'].includes(data.subscription.status)}
      <div class="notice">
        Your access is active{data.subscription.current_period_end
          ? ` through ${formatDate(data.subscription.current_period_end)}`
          : ''}.
      </div>
      <a class="button" href={`/creators/${data.profile.handle}`}>View projects</a>
    {:else}
      <div class="pending">
        <span class="label">Payments are coming later</span>
        <p>
          Checkout is not open yet. The access gate is live and can be granted manually during
          this preview.
        </p>
      </div>
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
</style>
