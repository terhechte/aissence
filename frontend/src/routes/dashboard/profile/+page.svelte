<script lang="ts">
  import { enhance } from '$app/forms';

  let { data, form } = $props();
  const profileValues = $derived((form as any)?.profileValues ?? {});
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
  }
</style>
