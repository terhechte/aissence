<script lang="ts">
  import ThemeToggle from './ThemeToggle.svelte';

  let { email } = $props<{ email?: string }>();
  let open = $state(false);
  let menu: HTMLDivElement;

  function toggle() {
    open = !open;
  }

  $effect(() => {
    if (!open) return;

    function dismiss(event: MouseEvent) {
      if (!menu.contains(event.target as Node)) open = false;
    }
    function escape(event: KeyboardEvent) {
      if (event.key === 'Escape') open = false;
    }

    document.addEventListener('click', dismiss);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('click', dismiss);
      document.removeEventListener('keydown', escape);
    };
  });
</script>

<div class="account-menu" bind:this={menu}>
  <button
    class="account-trigger"
    type="button"
    onclick={toggle}
    aria-label="Account menu"
    aria-haspopup="menu"
    aria-expanded={open}
  >
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="8" r="3.5"></circle>
      <path d="M5 20c.5-4 3.1-6 7-6s6.5 2 7 6"></path>
    </svg>
  </button>

  {#if open}
    <div class="account-dropdown" role="menu">
      {#if email}<p class="account-email" title={email}>{email}</p>{/if}
      <a href="/account" role="menuitem" onclick={() => (open = false)}>Account</a>
      <a href="/dashboard/profile" role="menuitem" onclick={() => (open = false)}>Creator Profile</a>
      <div role="menuitem"><ThemeToggle menu /></div>
      <form method="POST" action="/auth/signout">
        <button type="submit" role="menuitem">Sign out</button>
      </form>
    </div>
  {/if}
</div>

<style>
  .account-menu {
    position: relative;
  }
  .account-trigger {
    width: 38px;
    height: 38px;
    display: grid;
    place-items: center;
    border: 1px solid var(--rule2);
    border-radius: 50%;
    background: transparent;
    color: var(--body);
    cursor: pointer;
  }
  .account-trigger:hover,
  .account-trigger[aria-expanded='true'] {
    border-color: var(--ink);
    color: var(--ink);
  }
  svg {
    width: 19px;
    height: 19px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: 1.5;
  }
  .account-dropdown {
    position: absolute;
    z-index: 20;
    top: calc(100% + 10px);
    right: 0;
    width: 230px;
    border: 1px solid var(--rule2);
    background: var(--paper);
    box-shadow: 0 16px 40px rgb(0 0 0 / 12%);
    padding: 7px;
  }
  .account-email {
    overflow: hidden;
    margin: 0 7px 6px;
    padding: 8px 0 12px;
    border-bottom: 1px solid var(--rule);
    color: var(--muted);
    font: 400 9px/1.2 'JetBrains Mono', monospace;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .account-menu .account-dropdown a,
  .account-menu .account-dropdown form button {
    width: 100%;
    display: block;
    border: 0;
    padding: 12px 14px;
    background: transparent;
    color: var(--body);
    cursor: pointer;
    font: 400 11px/1.2 'JetBrains Mono', monospace;
    letter-spacing: normal;
    text-align: left;
    text-decoration: none;
    text-transform: none;
  }
  .account-menu .account-dropdown a:hover,
  .account-menu .account-dropdown form button:hover {
    background: var(--panel2);
    color: var(--ink);
  }
  form {
    margin: 0;
    border-top: 1px solid var(--rule);
  }
</style>
