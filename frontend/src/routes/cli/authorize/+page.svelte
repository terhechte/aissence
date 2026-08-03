<script lang="ts">
  import { enhance } from '$app/forms';

  let { data, form } = $props();
  let pending = $state(false);
</script>

<svelte:head>
  <title>Authorize the Aissence CLI</title>
  <meta name="referrer" content="no-referrer" />
</svelte:head>

<div class="page-shell authorize-wrap">
  <section>
    <p class="eyebrow">CLI authorization</p>
    {#if form?.authorized}
      <h1>CLI connected.</h1>
      <p>The Aissence CLI is now signed in. You can close this page and return to your terminal.</p>
      <a class="button secondary" href="/dashboard">Return to dashboard</a>
    {:else}
      <h1>Connect your terminal.</h1>
      <p>
        Authorize the CLI as <strong>{data.accountLabel}</strong>. It will be able to create and
        synchronize ledgers owned by this account.
      </p>
      <form
        method="POST"
        use:enhance={() => {
          pending = true;
          return async ({ update }) => {
            await update();
            pending = false;
          };
        }}
      >
        <input type="hidden" name="request_id" value={data.requestId} />
        <input type="hidden" name="browser_code" value={data.browserCode} />
        {#if form?.error}<p class="error">{form.error}</p>{/if}
        <button class="button" disabled={pending}>
          {pending ? 'Authorizing…' : 'Authorize CLI'}
        </button>
      </form>
      <p class="safety">Only continue if you just started this login from your own terminal.</p>
    {/if}
  </section>
</div>

<style>
  .authorize-wrap {
    min-height: calc(100vh - 79px);
    display: grid;
    place-items: center;
    padding-block: 70px;
  }
  section {
    width: min(100%, 700px);
    border-top: 1px solid var(--ink);
    padding-top: 28px;
  }
  h1 {
    margin: 18px 0 24px;
    font: 400 clamp(64px, 9vw, 108px) / 0.9 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.03em;
  }
  section > p:not(.eyebrow, .safety, .error) {
    max-width: 590px;
    color: var(--body);
    font-size: 21px;
    line-height: 1.5;
  }
  form {
    display: grid;
    justify-items: start;
    gap: 16px;
    margin-top: 30px;
  }
  .safety {
    margin-top: 18px;
    color: var(--muted);
    font: 400 10px/1.5 'JetBrains Mono', monospace;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
</style>
