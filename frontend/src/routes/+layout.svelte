<script lang="ts">
  import '../styles/global.css';
  import SiteHeader from '$lib/components/SiteHeader.svelte';

  let { children, data } = $props();
</script>

<svelte:head>
  <title>aissence — the reasoning behind the commit</title>
  <meta
    name="description"
    content="Publish the prompt history behind an AI-assisted software project."
  />
</svelte:head>

<SiteHeader claims={data.claims} />
<main>
  {@render children()}
</main>
