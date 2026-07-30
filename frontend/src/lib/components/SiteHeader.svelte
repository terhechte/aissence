<script lang="ts">
  import ThemeToggle from './ThemeToggle.svelte';

  let { claims = null } = $props<{ claims?: { sub: string; email?: string } | null }>();
</script>

<header class="site-header page-shell">
  <a class="wordmark" href="/" aria-label="Aissence home">aissence</a>
  <nav aria-label="Primary navigation">
    <a href="/#how">How it works</a>
    <a href="/#recent">Recent</a>
    {#if claims}
      <a href="/dashboard">Dashboard</a>
    {:else}
      <a href="/auth">Sign in</a>
    {/if}
    <ThemeToggle />
  </nav>
</header>
