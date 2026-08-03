<script lang="ts">
  import { formatDate } from '$lib/utils';

  let { data } = $props();

  const steps = [
    {
      n: '01',
      title: 'Drop the project record',
      body: 'Upload the .aissence file generated in your repository. It is checked before anything becomes public.'
    },
    {
      n: '02',
      title: 'Give the project context',
      body: 'Add the product URL, a concise description, and screenshots showing what those conversations produced.'
    },
    {
      n: '03',
      title: 'Review, then publish',
      body: 'Preview the full ledger privately. Publish only when the summaries and project presentation are ready.'
    }
  ];
</script>

<svelte:head>
  <title>aissence — host the history of what you built with AI</title>
</svelte:head>

<div class="page-shell home">
  <section class="hero">
    <div>
      <p class="eyebrow">Host the history of what you built with AI</p>
      <h1>The reasoning behind the commit.</h1>
      <p class="hero-copy">
        Publish a project as it actually happened: what you asked, what the model decided, and
        what the conversation became.
      </p>
      <div class="hero-actions">
        <a class="button" href={data.claims ? '/dashboard/projects/new' : '/auth'}>
          {data.claims ? 'Publish a project' : 'Create your ledger'}
        </a>
        <a class="text-link" href="#how">See how it works ↓</a>
      </div>
      {#if !data.configured}
        <p class="setup-note">Local preview mode — connect Supabase to enable accounts and publishing.</p>
      {/if}
    </div>
    <aside class="hero-ledger" aria-label="What a ledger contains">
      <p class="label">Inside every ledger</p>
      <dl>
        <div><dt>Asked</dt><dd>The human request, summarized for every commit.</dd></div>
        <div><dt>Decided</dt><dd>The assistant’s approach and important trade-offs.</dd></div>
        <div><dt>Built</dt><dd>The implementation, agents, token use, and committed change.</dd></div>
      </dl>
    </aside>
  </section>

  <section id="how" class="how">
    <div class="section-heading">
      <p class="label">How publishing works</p>
      <p>Private until you say otherwise</p>
    </div>
    <div class="steps">
      {#each steps as step}
        <article>
          <span>{step.n}</span>
          <h2>{step.title}</h2>
          <p>{step.body}</p>
        </article>
      {/each}
    </div>
  </section>

  <section id="recent" class="recent">
    <div class="section-heading">
      <p class="label">Recent projects</p>
      <div class="section-heading-actions">
        <p>Public ledgers, newest first</p>
        <a href="/projects">Browse all →</a>
      </div>
    </div>
    {#if data.projects.length}
      <div class="project-table">
        {#each data.projects as project}
          <a href={`/creators/${project.profiles.handle}/${project.slug}`}>
            <div>
              <h2>{project.name}</h2>
              {#if project.tagline}<p>{project.tagline}</p>{/if}
              {#if project.repository_label}<span>{project.repository_label}</span>{/if}
            </div>
            <span>{project.profiles.display_name}</span>
            <span>{project.commits?.[0]?.count ?? 0} entries</span>
            <time>{formatDate(project.published_at)}</time>
          </a>
        {/each}
      </div>
    {:else}
      <div class="empty-state">
        <p>No public ledgers yet.</p>
        <a href={data.claims ? '/dashboard/projects/new' : '/auth'}>Publish the first one →</a>
      </div>
    {/if}
  </section>

  <section class="closing">
    <p class="eyebrow">Keep the conversation with the code</p>
    <h2>Put your project’s reasoning somewhere it survives.</h2>
    <a class="button" href={data.claims ? '/dashboard/projects/new' : '/auth'}>
      {data.claims ? 'New project' : 'Sign in with email'}
    </a>
  </section>
</div>

<style>
  .home {
    padding-bottom: 110px;
  }
  .hero {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(280px, 0.75fr);
    gap: clamp(44px, 8vw, 100px);
    padding: 88px 0 100px;
    align-items: end;
  }
  h1 {
    max-width: 780px;
    margin: 18px 0 28px;
    font: 400 clamp(72px, 10vw, 132px) / 0.87 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.035em;
  }
  .hero-copy {
    max-width: 650px;
    margin: 0;
    font: 300 clamp(21px, 3vw, 29px) / 1.42 Newsreader, serif;
    color: var(--body2);
  }
  .hero-actions {
    display: flex;
    align-items: center;
    gap: 25px;
    margin-top: 34px;
    flex-wrap: wrap;
  }
  .text-link {
    font-size: 15px;
    color: var(--body);
  }
  .setup-note {
    color: var(--muted);
    font: 400 10px/1.5 'JetBrains Mono', monospace;
    margin-top: 18px;
  }
  .hero-ledger {
    border-top: 1px solid var(--ink);
    padding-top: 14px;
  }
  .hero-ledger dl {
    margin: 18px 0 0;
  }
  .hero-ledger dl div {
    padding: 16px 0;
    border-bottom: 1px solid var(--rule);
  }
  .hero-ledger dt {
    font: 400 25px/1 'Instrument Serif', Georgia, serif;
    color: var(--accent);
  }
  .hero-ledger dd {
    margin: 7px 0 0;
    color: var(--soft);
    line-height: 1.45;
  }
  .how,
  .recent {
    padding-top: 20px;
    margin-top: 30px;
    border-top: 1px solid var(--ink);
  }
  .section-heading {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 20px;
    margin-bottom: 42px;
    color: var(--muted);
    font-style: italic;
  }
  .section-heading p {
    margin: 0;
  }
  .section-heading .label {
    font-style: normal;
  }
  .section-heading-actions {
    display: flex;
    align-items: baseline;
    gap: 20px;
  }
  .section-heading-actions a {
    color: var(--accent);
    font-style: normal;
    text-decoration: none;
  }
  .steps {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 46px;
    padding-bottom: 90px;
  }
  .steps article > span {
    display: block;
    color: var(--accent);
    font: 500 11px/1 'JetBrains Mono', monospace;
    margin-bottom: 24px;
  }
  .steps h2 {
    font: 400 35px/1.05 'Instrument Serif', Georgia, serif;
    margin: 0 0 14px;
  }
  .steps p {
    color: var(--body);
    font-size: 17px;
    line-height: 1.55;
    margin: 0;
  }
  .project-table {
    border-top: 1px solid var(--rule);
    margin-bottom: 90px;
  }
  .project-table > a {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 160px 100px 120px;
    gap: 22px;
    align-items: center;
    padding: 20px 8px 20px 0;
    border-bottom: 1px solid var(--rule);
    text-decoration: none;
  }
  .project-table > a:hover {
    background: var(--panel2);
  }
  .project-table h2 {
    font: 400 26px/1.1 'Instrument Serif', Georgia, serif;
    margin: 0;
  }
  .project-table p {
    color: var(--soft);
    margin: 5px 0 8px;
  }
  .project-table span,
  .project-table time {
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .project-table div > span {
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .closing {
    margin-top: 95px;
    padding: 78px clamp(30px, 8vw, 100px);
    background: var(--panel);
    text-align: center;
  }
  .closing h2 {
    max-width: 800px;
    margin: 18px auto 30px;
    font: 400 clamp(48px, 7vw, 82px) / 0.98 'Instrument Serif', Georgia, serif;
  }
  @media (max-width: 820px) {
    .hero {
      grid-template-columns: 1fr;
      padding-top: 60px;
    }
    .steps {
      grid-template-columns: 1fr;
      gap: 34px;
    }
    .project-table > a {
      grid-template-columns: 1fr auto;
    }
    .project-table > a > span:first-of-type,
    .project-table > a > span:last-of-type {
      display: none;
    }
    .section-heading {
      align-items: flex-start;
      flex-direction: column;
      gap: 8px;
    }
  }
</style>
