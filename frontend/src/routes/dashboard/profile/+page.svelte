<script lang="ts">
  import { enhance } from '$app/forms';

  let { data, form } = $props();
  const profileValues = $derived((form as any)?.profileValues ?? {});
  const planValues = $derived((form as any)?.planValues ?? {});

  function planPrice(interval: string) {
    const plan = data.plans.find((item: any) => item.interval === interval);
    return plan ? (Number(plan.price_cents) / 100).toFixed(2) : '';
  }

  const kycStatus = $derived(data.account?.kyc_status ?? 'not_started');
</script>

<svelte:head><title>Edit creator profile — aissence</title></svelte:head>

<div class="page-shell profile-admin">
  <header>
    <div>
      <p class="eyebrow">Creator profile</p>
      <h1>How readers find you.</h1>
      <p>Keep the public identity behind your ledgers current.</p>
    </div>
    <a class="button secondary" href="/dashboard">Back to dashboard</a>
  </header>

  {#if form?.error}<p class="error">{form.error}</p>{/if}
  {#if form?.saved}<p class="notice">Creator profile saved.</p>{/if}

  <section class="panel profile-panel">
    <div class="section-head">
      <span>01</span>
      <div><h2>Public details</h2><p>This is shown wherever readers see your creator profile.</p></div>
    </div>

    <form method="POST" action="?/saveProfile" enctype="multipart/form-data" use:enhance>
      <div class="form-grid">
        <div class="field">
          <label for="display_name">Display name</label>
          <input id="display_name" name="display_name" required value={profileValues.display_name ?? data.profile.display_name} />
        </div>
        <div class="field">
          <label for="handle">Creator handle</label>
          <input id="handle" name="handle" required value={profileValues.handle ?? data.profile.handle} />
          <small>Lowercase letters, numbers, and hyphens.</small>
        </div>
        <div class="field wide">
          <label for="bio">Short bio <span>(optional)</span></label>
          <textarea id="bio" name="bio">{profileValues.bio ?? data.profile.bio ?? ''}</textarea>
        </div>
        <div class="field wide avatar-field">
          <label for="avatar">Profile image <span>(optional)</span></label>
          {#if data.profile.avatar_url}
            <div class="current-avatar">
              <img src={data.profile.avatar_url} alt="" />
              <span>Current profile image</span>
            </div>
          {/if}
          <input id="avatar" name="avatar" type="file" accept="image/jpeg,image/png,image/webp" />
          <small>JPEG, PNG, or WebP · up to 5 MB · leave blank to keep the current image</small>
        </div>
      </div>
      <button class="button">Save creator profile</button>
    </form>
  </section>

  <section class="panel plans-panel">
    <div class="section-head">
      <span>02</span>
      <div>
        <h2>Subscription plans</h2>
        <p>One creator subscription unlocks every current and future published ledger.</p>
      </div>
    </div>

    <div class="provider-status">
      <strong>Payment provider status</strong>
      <span>{kycStatus === 'verified' ? 'KYC verified' : 'KYC not yet verified'}</span>
      <small>Plans can be prepared now. A payment provider must verify you before checkout can accept money.</small>
    </div>

    {#if form?.planError}<p class="error">{form.planError}</p>{/if}
    {#if form?.plansSaved}<p class="notice">Subscription plans saved.</p>{/if}

    <form class="plans-form" method="POST" action="?/savePlans" use:enhance>
      <div class="plan-fields">
        <div class="field">
          <label for="monthly_price">Monthly price (USD)</label>
          <input id="monthly_price" name="monthly_price" type="number" min="0.01" step="0.01" placeholder="10.00" value={planValues.monthly_price ?? planPrice('monthly')} />
          <small>Leave blank to hide the monthly option.</small>
        </div>
        <div class="field">
          <label for="annual_price">Annual price (USD)</label>
          <input id="annual_price" name="annual_price" type="number" min="0.01" step="0.01" placeholder="100.00" value={planValues.annual_price ?? planPrice('annual')} />
          <small>Leave blank to hide the annual option.</small>
        </div>
      </div>
      <button class="button">Save plans</button>
    </form>
  </section>
</div>

<style>
  .profile-admin {
    padding-block: 55px 120px;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: 30px;
    margin-bottom: 70px;
  }
  header h1 {
    margin: 16px 0;
    font: 400 clamp(62px, 9vw, 104px) / 0.9 'Instrument Serif', Georgia, serif;
  }
  header p:last-child {
    color: var(--muted);
  }
  .profile-panel {
    max-width: 900px;
  }
  .plans-panel {
    max-width: 900px;
  }
  .profile-panel .form-grid > .field {
    align-content: start;
  }
  .section-head {
    display: grid;
    grid-template-columns: 58px 1fr;
    margin-bottom: 28px;
  }
  .section-head > span {
    color: var(--accent);
    font: 500 11px/1 'JetBrains Mono', monospace;
  }
  .section-head h2 {
    margin: 0;
    font: 400 35px/1 'Instrument Serif', Georgia, serif;
  }
  .section-head p {
    margin: 8px 0 0;
    color: var(--muted);
  }
  form {
    display: grid;
    gap: 24px;
  }
  form > button {
    justify-self: start;
  }
  .field span {
    color: var(--faint);
  }
  .field small {
    color: var(--muted);
  }
  .avatar-field {
    gap: 9px;
  }
  .provider-status {
    display: grid;
    gap: 5px;
    margin-bottom: 24px;
    padding: 16px 18px;
    background: var(--panel2);
  }
  .provider-status strong {
    font-weight: 400;
  }
  .provider-status span,
  .provider-status small {
    color: var(--muted);
  }
  .plans-form {
    display: grid;
    gap: 24px;
  }
  .plan-fields {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 20px;
  }
  .current-avatar {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--muted);
    font: 400 10px/1.3 'JetBrains Mono', monospace;
  }
  .current-avatar img {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    object-fit: cover;
    border: 1px solid var(--rule2);
  }
  @media (max-width: 720px) {
    header {
      align-items: flex-start;
      flex-direction: column;
    }
    .plan-fields {
      grid-template-columns: 1fr;
    }
  }
</style>
