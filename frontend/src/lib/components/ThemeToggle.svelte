<script lang="ts">
  let { menu = false } = $props<{ menu?: boolean }>();
  let dark = $state(false);

  function sync() {
    dark = document.documentElement.dataset.theme === 'dark';
  }

  function toggle() {
    dark = !dark;
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    localStorage.setItem('aissence-theme', dark ? 'dark' : 'light');
  }

  $effect(() => {
    sync();
  });
</script>

<button
  class:theme-toggle={!menu}
  class:menu-theme-toggle={menu}
  type="button"
  onclick={toggle}
  aria-label={`Switch to ${dark ? 'light' : 'dark'} mode`}
>
  <span>{dark ? 'Light mode' : 'Dark mode'}</span>
  {#if menu}<span class="theme-state">{dark ? 'Dark' : 'Light'}</span>{/if}
</button>

<style>
  .menu-theme-toggle {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: 0;
    padding: 12px 14px;
    background: transparent;
    color: var(--body);
    cursor: pointer;
    font: 400 11px/1.2 'JetBrains Mono', monospace;
    text-align: left;
  }
  .menu-theme-toggle:hover {
    background: var(--panel2);
    color: var(--ink);
  }
  .theme-state {
    color: var(--muted);
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
</style>
