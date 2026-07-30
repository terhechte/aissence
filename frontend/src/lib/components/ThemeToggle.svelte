<script lang="ts">
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

<button class="theme-toggle" type="button" onclick={toggle} aria-label="Toggle color theme">
  {dark ? 'light' : 'dark'}
</button>
