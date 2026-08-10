<script lang="ts">
  import Ledger from '$lib/components/Ledger.svelte';

  let { data } = $props();
  let previewMode = $state<'public' | 'subscriber'>('public');
</script>

<svelte:head>
  <title>{data.project.name} — local aissence preview</title>
  <meta name="robots" content="noindex,nofollow" />
</svelte:head>

<div class="preview-banner">
  <span>Local development preview — kept in memory for 24 hours</span>
  <div class="preview-switch" role="group" aria-label="Preview audience">
    <button
      class:active={previewMode === 'public'}
      aria-pressed={previewMode === 'public'}
      onclick={() => (previewMode = 'public')}
    >Public</button>
    <button
      class:active={previewMode === 'subscriber'}
      aria-pressed={previewMode === 'subscriber'}
      onclick={() => (previewMode = 'subscriber')}
    >Subscriber</button>
  </div>
</div>

{#if data.warnings.length}
  <div class="warnings">
    {#each data.warnings as warning}<p>{warning}</p>{/each}
  </div>
{/if}

<Ledger
  profile={data.profile}
  project={data.project}
  commits={data.commits}
  screenshots={data.screenshots}
  isEntitled={previewMode === 'subscriber'}
  isOwner={true}
/>

<style>
  .preview-banner {
    position: sticky;
    top: 0;
    z-index: 10;
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 24px;
    min-height: 48px;
    padding: 7px 20px;
    background: var(--accent);
    color: var(--paper);
    font: 500 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  .preview-switch {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid color-mix(in srgb, var(--paper) 55%, transparent);
  }
  .preview-switch button {
    border: 0;
    padding: 7px 10px;
    background: transparent;
    color: var(--paper);
    font: inherit;
    text-transform: inherit;
    letter-spacing: inherit;
    cursor: pointer;
  }
  .preview-switch button.active {
    background: var(--paper);
    color: var(--accent);
  }
  .warnings {
    padding: 10px 24px;
    background: #fff4d6;
    color: #5c4200;
    font: 12px/1.5 'JetBrains Mono', monospace;
  }
  .warnings p { margin: 0; }
  @media (max-width: 620px) {
    .preview-banner {
      flex-direction: column;
      gap: 7px;
      padding-block: 9px;
    }
  }
</style>
