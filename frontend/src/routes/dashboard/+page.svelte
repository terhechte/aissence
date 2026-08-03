<script lang="ts">
  import { enhance } from '$app/forms';
  import { formatDate } from '$lib/utils';
  let { data, form } = $props();
  const profileValues = $derived((form as any)?.profileValues ?? {});
</script>

<svelte:head><title>Dashboard — aissence</title></svelte:head>

<div class="page-shell dashboard">
  <div class="dashboard-head">
    <div>
      <p class="eyebrow">Creator dashboard</p>
      <h1>{data.profile ? `Good to see you, ${data.profile.display_name}.` : 'Choose your creator identity.'}</h1>
    </div>
  </div>

  {#if !data.profile}
    <section class="panel onboarding">
      <p>Your public creator profile owns projects and is the unit readers subscribe to.</p>
      <form method="POST" action="?/saveProfile" enctype="multipart/form-data" use:enhance>
        <div class="form-grid">
          <div class="field">
            <label for="display_name">Display name</label>
            <input id="display_name" name="display_name" required value={profileValues.display_name ?? ''} />
          </div>
          <div class="field">
            <label for="handle">Creator handle</label>
            <input id="handle" name="handle" required placeholder="your-name" value={profileValues.handle ?? ''} />
          </div>
          <div class="field wide">
            <label for="bio">Short bio <span>(optional)</span></label>
            <textarea id="bio" name="bio">{profileValues.bio ?? ''}</textarea>
          </div>
          <div class="field wide">
            <label for="avatar">Profile image <span>(optional)</span></label>
            <input id="avatar" name="avatar" type="file" accept="image/jpeg,image/png,image/webp" />
            <small>JPEG, PNG, or WebP · up to 5 MB</small>
          </div>
        </div>
        {#if form?.profileError}<p class="error">{form.profileError}</p>{/if}
        <button class="button">Create creator profile</button>
      </form>
    </section>
  {:else}
    <section class="projects">
      <div class="section-title">
        <div>
          <p class="label">Your projects</p>
          <p>{data.projects.length} {data.projects.length === 1 ? 'ledger' : 'ledgers'}</p>
        </div>
        <a class="button" href="/dashboard/projects/new">New project</a>
      </div>
      {#if data.projects.length}
        <div class="project-list">
          {#each data.projects as project}
            <a href={`/dashboard/projects/${project.id}`}>
              <div>
                <h2>{project.name}</h2>
                {#if project.tagline}<p>{project.tagline}</p>{/if}
              </div>
              <span class:published={project.visibility === 'published'}>{project.visibility}</span>
              <span>
                {project.imports?.[0]?.commit_count ?? 0} entries
              </span>
              <time>{formatDate(project.updated_at)}</time>
            </a>
          {/each}
        </div>
      {:else}
        <div class="empty-state">
          <p>Your first ledger begins with a single project file.</p>
          <a href="/dashboard/projects/new">Upload a .aissence file →</a>
        </div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .dashboard {
    padding-block: 58px 110px;
  }
  .dashboard-head,
  .section-title {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 30px;
  }
  h1 {
    margin: 16px 0 55px;
    font: 400 clamp(48px, 7vw, 80px) / 0.98 'Instrument Serif', Georgia, serif;
  }
  .onboarding {
    max-width: 820px;
  }
  .onboarding > p {
    font-size: 20px;
    color: var(--body);
    max-width: 600px;
    margin-top: 0;
  }
  .onboarding form {
    display: grid;
    gap: 22px;
  }
  .field span {
    color: var(--faint);
  }
  .field small {
    color: var(--muted);
  }
  .section-title {
    align-items: center;
    border-bottom: 1px solid var(--ink);
    padding-bottom: 15px;
  }
  .section-title p {
    margin: 0 0 6px;
    color: var(--muted);
  }
  .project-list a {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 100px 100px 120px;
    align-items: center;
    gap: 22px;
    border-bottom: 1px solid var(--rule);
    padding: 20px 8px 20px 0;
    text-decoration: none;
  }
  .project-list a:hover {
    background: var(--panel2);
  }
  .project-list h2 {
    font: 400 27px/1.15 'Instrument Serif', Georgia, serif;
    margin: 0;
  }
  .project-list p {
    color: var(--soft);
    margin: 5px 0 0;
  }
  .project-list span,
  .project-list time {
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .project-list span:first-of-type {
    text-transform: uppercase;
    letter-spacing: 0.09em;
  }
  .project-list span.published {
    color: var(--accent);
  }
  .empty-state {
    margin-top: 30px;
  }
  @media (max-width: 720px) {
    .dashboard-head {
      align-items: flex-start;
      flex-direction: column;
    }
    .project-list a {
      grid-template-columns: 1fr auto;
    }
    .project-list a > span:last-of-type,
    .project-list time {
      display: none;
    }
  }
</style>
