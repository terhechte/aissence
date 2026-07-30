<script lang="ts">
  import { page } from '$app/state';
</script>

<svelte:head><title>{page.status} — aissence</title></svelte:head>

<div class="page-shell error-page">
  <p class="eyebrow">Error {page.status}</p>
  <h1>{page.error?.message || 'Something went missing.'}</h1>
  <p>The ledger could not be opened. It may be private, unpublished, or no longer available.</p>
  <a class="button secondary" href="/">Return home</a>
</div>

<style>
  .error-page {
    min-height: calc(100vh - 79px);
    display: grid;
    align-content: center;
    justify-items: start;
    padding-block: 70px;
  }
  h1 {
    max-width: 800px;
    margin: 18px 0;
    font: 400 clamp(60px, 9vw, 105px) / 0.92 'Instrument Serif', Georgia, serif;
  }
  p:not(.eyebrow) {
    max-width: 550px;
    color: var(--body);
    font-size: 20px;
    line-height: 1.5;
  }
</style>
