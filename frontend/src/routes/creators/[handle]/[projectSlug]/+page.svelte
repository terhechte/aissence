<script lang="ts">
  import Ledger from '$lib/components/Ledger.svelte';
  let { data } = $props();
</script>

<svelte:head>
  <title>{data.project.name} — {data.profile.display_name} on aissence</title>
  <meta name="description" content={data.project.tagline} />
</svelte:head>

{#if data.project.visibility === 'draft'}
  <div class="draft-banner">Private creator preview — this ledger is not published.</div>
{/if}

<Ledger
  profile={data.profile}
  project={data.project}
  commits={data.commits}
  screenshots={data.screenshots}
  isOwner={data.isOwner}
  isEntitled={data.isEntitled}
/>

<style>
  .draft-banner {
    position: sticky;
    top: 0;
    z-index: 10;
    padding: 9px 20px;
    text-align: center;
    background: var(--accent);
    color: var(--paper);
    font: 500 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
</style>
