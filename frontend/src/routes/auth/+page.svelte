<script lang="ts">
  import { enhance } from '$app/forms';
  let { data, form } = $props();
  let pending = $state(false);
</script>

<svelte:head><title>Sign in — aissence</title></svelte:head>

<div class="page-shell auth-wrap">
  <section>
    <p class="eyebrow">Creator access</p>
    <h1>One link. No password.</h1>
    <p>Enter your email and we’ll send a secure sign-in link for your creator dashboard.</p>
  </section>
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
    <input type="hidden" name="next" value={form?.next ?? data.next} />
    {#if form?.success}
      <div class="notice">
        Check <strong>{form.email}</strong>. The link in that email will bring you back here.
      </div>
    {:else}
      <div class="field">
        <label for="email">Email address</label>
        <input id="email" name="email" type="email" autocomplete="email" value={form?.email ?? ''} required />
      </div>
      {#if form?.error}<p class="error">{form.error}</p>{/if}
      {#if !data.configured}
        <p class="error">Copy <code>.env.example</code> to <code>.env</code> and add your Supabase keys.</p>
      {/if}
      <button class="button" disabled={pending || !data.configured}>
        {pending ? 'Sending…' : 'Email me a sign-in link'}
      </button>
    {/if}
  </form>
</div>

<style>
  .auth-wrap {
    min-height: calc(100vh - 79px);
    display: grid;
    grid-template-columns: 1.2fr 0.8fr;
    gap: 80px;
    align-items: center;
    padding-block: 70px;
  }
  h1 {
    font: 400 clamp(64px, 9vw, 112px) / 0.9 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.03em;
    margin: 18px 0 24px;
  }
  section > p:last-child {
    max-width: 510px;
    color: var(--body);
    font-size: 21px;
    line-height: 1.5;
  }
  form {
    border-top: 1px solid var(--ink);
    padding-top: 24px;
    display: grid;
    gap: 18px;
  }
  code {
    font-family: 'JetBrains Mono', monospace;
  }
  @media (max-width: 760px) {
    .auth-wrap {
      grid-template-columns: 1fr;
      gap: 38px;
      align-items: start;
    }
  }
</style>
