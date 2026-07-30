<script lang="ts">
  import { formatDate } from '$lib/utils';
  let { data } = $props();
</script>

<svelte:head>
  <title>{data.profile.display_name} — aissence</title>
  <meta name="description" content={data.profile.bio || `Projects published by ${data.profile.display_name}`} />
</svelte:head>

<div class="page-shell creator">
  <header>
    <div class="identity">
      <p class="eyebrow">Creator ledger</p>
      {#if data.profile.avatar_url}
        <img class="avatar" src={data.profile.avatar_url} alt="" />
      {/if}
      <h1>{data.profile.display_name}</h1>
      <p>{data.profile.bio || 'The reasoning behind the projects.'}</p>
    </div>
    <a class="button secondary" href={`/creators/${data.profile.handle}/subscribe`}>Subscribe to prompts</a>
  </header>
  <section>
    <div class="section-head"><span>Published projects</span><span>{data.projects.length} total</span></div>
    {#if data.projects.length}
      <div class="projects">
        {#each data.projects as project}
          <a href={`/creators/${data.profile.handle}/${project.slug}`}>
            <div><h2>{project.name}</h2><p>{project.tagline}</p>{#if project.repository_label}<span>{project.repository_label}</span>{/if}</div>
            <span>{project.commits?.[0]?.count ?? 0} entries</span>
            <time>{formatDate(project.published_at)}</time>
          </a>
        {/each}
      </div>
    {:else}
      <div class="empty-state">No published projects yet.</div>
    {/if}
  </section>
</div>

<style>
  .creator {
    padding-block: 70px 120px;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: 30px;
    padding-bottom: 70px;
  }
  h1 {
    font: 400 clamp(76px, 12vw, 140px) / 0.86 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.035em;
    margin: 20px 0;
  }
  header p:last-child {
    max-width: 600px;
    font-size: 22px;
    color: var(--body);
  }
  .avatar {
    width: clamp(96px, 13vw, 160px);
    aspect-ratio: 1;
    object-fit: cover;
    border-radius: 50%;
    margin-top: 24px;
  }
  section {
    border-top: 1px solid var(--ink);
  }
  .section-head {
    display: flex;
    justify-content: space-between;
    padding: 14px 0;
    color: var(--muted);
    font: 400 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  .projects > a {
    display: grid;
    grid-template-columns: 1fr 100px 130px;
    gap: 25px;
    align-items: center;
    padding: 22px 8px 22px 0;
    border-top: 1px solid var(--rule);
    text-decoration: none;
  }
  .projects > a:hover {
    background: var(--panel2);
  }
  h2 {
    margin: 0;
    font: 400 31px/1.1 'Instrument Serif', Georgia, serif;
  }
  .projects p {
    color: var(--soft);
    margin: 6px 0 10px;
  }
  .projects span,
  .projects time {
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .projects div > span {
    color: var(--accent);
    text-transform: uppercase;
  }
  @media (max-width: 700px) {
    header {
      align-items: flex-start;
      flex-direction: column;
    }
    .projects > a {
      grid-template-columns: 1fr auto;
    }
    .projects > a > span {
      display: none;
    }
  }
</style>
