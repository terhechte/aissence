<script lang="ts">
  import { enhance } from '$app/forms';

  let { data, form } = $props();
  let pending = $state(false);
</script>

<svelte:head><title>Account profile — aissence</title></svelte:head>

<div class="page-shell account-page">
  <header>
    <div>
      <p class="eyebrow">Account</p>
      <h1>Your profile.</h1>
      <p>View your sign-in details and keep your account secure.</p>
    </div>
    <a class="button secondary" href="/dashboard">Back to dashboard</a>
  </header>

  <div class="account-grid">
    <section class="panel">
      <p class="section-number">01</p>
      <h2>Profile</h2>
      <div class="detail">
        <span>Email address</span>
        <strong>{data.email}</strong>
      </div>
      <p class="help">This is the address you use to sign in to aissence.</p>
    </section>

    <section class="panel">
      <p class="section-number">02</p>
      <h2>Set or reset password</h2>
      <p class="help">Magic-link sign-in will keep working after you add or change a password.</p>

      {#if form?.passwordSaved}<p class="notice">Your password has been updated.</p>{/if}
      {#if form?.error}<p class="error">{form.error}</p>{/if}

      <form
        method="POST"
        action="?/password"
        use:enhance={() => {
          pending = true;
          return async ({ update }) => {
            await update({ reset: false });
            pending = false;
          };
        }}
      >
        <div class="field">
          <label for="password">New password</label>
          <input id="password" name="password" type="password" autocomplete="new-password" minlength="8" required />
        </div>
        <div class="field">
          <label for="password_confirmation">Confirm new password</label>
          <input id="password_confirmation" name="password_confirmation" type="password" autocomplete="new-password" minlength="8" required />
        </div>
        <button class="button" disabled={pending}>{pending ? 'Updating…' : 'Update password'}</button>
      </form>
    </section>
  </div>
</div>

<style>
  .account-page { padding-block: 55px 120px; }
  header { display: flex; justify-content: space-between; align-items: flex-end; gap: 30px; margin-bottom: 70px; }
  header h1 { margin: 16px 0; font: 400 clamp(62px, 9vw, 104px) / 0.9 'Instrument Serif', Georgia, serif; }
  header p:last-child, .help { color: var(--muted); }
  .account-grid { display: grid; grid-template-columns: minmax(0, .8fr) minmax(0, 1.2fr); gap: 24px; max-width: 950px; }
  .section-number { margin: 0 0 16px; color: var(--accent); font: 500 11px/1 'JetBrains Mono', monospace; }
  h2 { margin: 0 0 25px; font: 400 35px/1 'Instrument Serif', Georgia, serif; }
  .detail { display: grid; gap: 8px; padding-block: 14px; border-block: 1px solid var(--rule); }
  .detail span { color: var(--muted); font: 400 10px/1.2 'JetBrains Mono', monospace; letter-spacing: .1em; text-transform: uppercase; }
  .detail strong { overflow-wrap: anywhere; font-weight: 400; }
  .help { line-height: 1.45; }
  form { display: grid; gap: 18px; margin-top: 24px; }
  form .button { justify-self: start; }
  @media (max-width: 720px) {
    header { align-items: flex-start; flex-direction: column; margin-bottom: 42px; }
    .account-grid { grid-template-columns: 1fr; }
  }
</style>
