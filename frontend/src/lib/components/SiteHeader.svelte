<script lang="ts">
  import ThemeToggle from './ThemeToggle.svelte';
  import AccountMenu from './AccountMenu.svelte';

  let { claims = null } = $props<{ claims?: { sub: string; email?: string } | null }>();
</script>

<header class="site-header page-shell">
  <a class="wordmark" href="/" aria-label="Aissence home">
    <img class="wordmark-icon wordmark-icon-light" src="/icon-black.png" alt="" aria-hidden="true" />
    <img class="wordmark-icon wordmark-icon-dark" src="/icon-white.png" alt="" aria-hidden="true" />
    <span>aissence</span>
  </a>
  <nav aria-label="Primary navigation">
    <a class="projects-link" href="/projects">Projects</a>
    <a href="/#how">How it works</a>
    {#if claims}
      <a href="/dashboard">Dashboard</a>
      <a href="/dashboard/subscriptions">Subscriptions</a>
      <AccountMenu email={claims.email} />
    {:else}
      <a href="/auth">Sign in</a>
      <ThemeToggle />
    {/if}
  </nav>
</header>
