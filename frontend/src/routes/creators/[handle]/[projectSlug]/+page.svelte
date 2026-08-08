<script lang="ts">
  import Ledger from '$lib/components/Ledger.svelte';
  let { data } = $props();
  let previewMode = $state<'public' | 'subscriber'>('public');
  const isPrivatePreview = $derived(data.project.visibility === 'draft' && data.isOwner);
  const hasSubscriberAccess = $derived(
    isPrivatePreview ? previewMode === 'subscriber' : data.isEntitled
  );
</script>

<svelte:head>
  <title>{data.project.name} — {data.profile.display_name} on aissence</title>
  <meta name="description" content={data.project.tagline || data.project.description || `The ${data.project.name} project ledger`} />
</svelte:head>

{#if data.project.visibility === 'draft'}
  <div class="draft-banner">
    <span>Private creator preview — this ledger is not published.</span>
    {#if data.isOwner}
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
        >Paying subscriber</button>
      </div>
    {/if}
  </div>
{/if}

<Ledger
  profile={data.profile}
  project={data.project}
  commits={data.commits}
  screenshots={data.screenshots}
  isEntitled={hasSubscriberAccess}
  isOwner={data.isOwner}
/>

<style>
  .draft-banner {
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
  @media (max-width: 620px) {
    .draft-banner {
      flex-direction: column;
      gap: 7px;
      padding-block: 9px;
    }
  }
</style>
