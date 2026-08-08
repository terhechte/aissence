<script lang="ts">
  import { formatDate } from '$lib/utils';

  let { data } = $props();
  let query = $state('');
  let kind = $state<'all' | 'source' | 'product'>('all');
  let creator = $state('all');

  const kinds = [
    { id: 'all', label: 'All ledgers' },
    { id: 'source', label: 'With source' },
    { id: 'product', label: 'With product' }
  ] as const;

  const creators = $derived(
    [...new Map(data.projects.map((project: any) => [project.profiles?.handle, project.profiles?.display_name]))]
      .filter(([handle, name]) => handle && name)
      .sort((a, b) => String(a[1]).localeCompare(String(b[1])))
  );

  const filteredProjects = $derived(
    data.projects.filter((project: any) => {
      const haystack = [
        project.name,
        project.tagline,
        project.description,
        project.summary_overview,
        project.repository_label,
        project.profiles?.display_name,
        project.profiles?.handle
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      const normalizedQuery = query.trim().toLowerCase();
      const matchesQuery = !normalizedQuery || haystack.includes(normalizedQuery);
      const matchesKind =
        kind === 'all' ||
        (kind === 'source' && (project.repository_label || project.repository_url)) ||
        (kind === 'product' && project.product_url);
      const matchesCreator = creator === 'all' || project.profiles?.handle === creator;

      return matchesQuery && matchesKind && matchesCreator;
    })
  );

  const hasFilters = $derived(Boolean(query.trim()) || kind !== 'all' || creator !== 'all');

  function clearFilters() {
    query = '';
    kind = 'all';
    creator = 'all';
  }
</script>

<svelte:head>
  <title>Projects — aissence</title>
  <meta
    name="description"
    content="Browse public aissence ledgers and explore the reasoning behind real projects."
  />
</svelte:head>

<div class="page-shell projects-page">
  <section class="directory-intro">
    <div>
      <p class="eyebrow">The public project index</p>
      <h1>Browse the work.<br /><em>Read the reasoning.</em></h1>
      <p class="intro-copy">
        Every ledger is a project told from the inside: the asks, decisions, tools, and changes
        that shaped what was built.
      </p>
    </div>
    <aside class="directory-note">
      <p class="label">How to explore</p>
      <p>Search by project, creator, or repository. Use the filters to find ledgers with a live product or public source.</p>
    </aside>
  </section>

  <section class="directory" aria-labelledby="directory-heading">
    <header class="directory-heading">
      <div>
        <p class="label">Project index</p>
        <h2 id="directory-heading">Public ledgers</h2>
      </div>
      <p class="result-count">
        {filteredProjects.length} of {data.projects.length} {data.projects.length === 1 ? 'ledger' : 'ledgers'}
      </p>
    </header>

    <form class="browse-controls" onsubmit={(event) => event.preventDefault()}>
      <label class="search-field">
        <span class="sr-only">Search projects</span>
        <span class="search-mark" aria-hidden="true">⌕</span>
        <input bind:value={query} type="search" placeholder="Search projects, creators, repositories…" />
      </label>
      <label class="creator-field">
        <span class="sr-only">Filter by creator</span>
        <select bind:value={creator} aria-label="Filter by creator">
          <option value="all">All creators</option>
          {#each creators as [handle, name]}
            <option value={handle}>{name}</option>
          {/each}
        </select>
      </label>
      <div class="kind-filters" aria-label="Filter by project type">
        {#each kinds as filter}
          <button
            type="button"
            class:active={kind === filter.id}
            aria-pressed={kind === filter.id}
            onclick={() => (kind = filter.id)}
          >
            {filter.label}
          </button>
        {/each}
      </div>
    </form>

    <div class="results-bar">
      <p>{hasFilters ? 'Filtered results' : 'Newest first'}</p>
      {#if hasFilters}
        <button type="button" onclick={clearFilters}>Clear filters ×</button>
      {/if}
    </div>

    {#if filteredProjects.length}
      <div class="project-grid">
        {#each filteredProjects as project, index}
          <a
            class="project-card"
            href={`/creators/${project.profiles.handle}/${project.slug}`}
            aria-label={`Read the ${project.name} ledger by ${project.profiles.display_name}`}
          >
            <div class="card-topline">
              <span class="card-index">{String(index + 1).padStart(2, '0')}</span>
              <span class="card-arrow" aria-hidden="true">↗</span>
            </div>
            <div class="card-body">
              <div class="card-tags">
                {#if project.repository_label}<span>Source</span>{/if}
                {#if project.product_url}<span>Product</span>{/if}
              </div>
              <h3>{project.name}</h3>
              {#if project.tagline || project.summary_overview}<p>{project.tagline || project.summary_overview}</p>{/if}
            </div>
            <div class="card-meta">
              <span>{project.profiles.display_name}</span>
              <span>{project.commits?.[0]?.count ?? 0} entries</span>
              <time>{formatDate(project.published_at, { month: 'short', year: 'numeric' })}</time>
            </div>
          </a>
        {/each}
      </div>
    {:else if data.projects.length}
      <div class="empty-state filtered-empty">
        <p>No ledgers match those filters.</p>
        <button type="button" onclick={clearFilters}>Show every project →</button>
      </div>
    {:else}
      <div class="empty-state">
        <p>{data.configured ? 'No public ledgers yet.' : 'The public index is waiting for its first ledger.'}</p>
        <a href={data.configured ? '/auth' : '/'}>{data.configured ? 'Publish the first one →' : 'Back to aissence →'}</a>
      </div>
    {/if}
  </section>
</div>

<style>
  .projects-page {
    padding-bottom: 120px;
  }

  .directory-intro {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(250px, 0.6fr);
    align-items: end;
    gap: clamp(40px, 10vw, 140px);
    padding: 92px 0 100px;
  }

  h1 {
    max-width: 780px;
    margin: 18px 0 28px;
    font: 400 clamp(68px, 10vw, 126px) / 0.86 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.04em;
  }

  h1 em {
    color: var(--accent);
  }

  .intro-copy {
    max-width: 610px;
    margin: 0;
    color: var(--body2);
    font: 300 clamp(20px, 2.7vw, 27px) / 1.4 Newsreader, serif;
  }

  .directory-note {
    border-top: 1px solid var(--ink);
    padding-top: 14px;
  }

  .directory-note p:last-child {
    max-width: 270px;
    margin: 18px 0 0;
    color: var(--soft);
    font-size: 18px;
    line-height: 1.45;
  }

  .directory {
    border-top: 1px solid var(--ink);
  }

  .directory-heading {
    display: flex;
    justify-content: space-between;
    align-items: end;
    gap: 20px;
    padding: 18px 0 28px;
  }

  .directory-heading .label {
    margin-bottom: 10px;
  }

  h2 {
    margin: 0;
    font: 400 42px/1 'Instrument Serif', Georgia, serif;
  }

  .result-count,
  .results-bar,
  .card-meta,
  .card-index,
  .card-tags,
  .kind-filters button,
  .creator-field select,
  .search-field input {
    font: 400 10px/1.35 'JetBrains Mono', monospace;
    letter-spacing: 0.07em;
  }

  .result-count {
    margin: 0 0 4px;
    color: var(--muted);
    text-transform: uppercase;
  }

  .browse-controls {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 210px;
    gap: 12px;
    padding: 16px 0;
    border-top: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
  }

  .search-field,
  .creator-field {
    display: flex;
    align-items: center;
    border: 1px solid var(--rule2);
    background: var(--panel);
  }

  .search-mark {
    padding-left: 14px;
    color: var(--accent);
    font: 400 24px/1 Georgia, serif;
  }

  .search-field input,
  .creator-field select {
    width: 100%;
    min-height: 48px;
    border: 0;
    padding: 0 14px;
    color: var(--ink);
    background: transparent;
    outline: 0;
    letter-spacing: 0;
  }

  .search-field input::placeholder {
    color: var(--muted);
  }

  .creator-field select {
    cursor: pointer;
    letter-spacing: 0.03em;
  }

  .kind-filters {
    grid-column: 1 / -1;
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .kind-filters button,
  .results-bar button,
  .filtered-empty button {
    border: 1px solid var(--rule2);
    padding: 10px 12px;
    color: var(--muted);
    background: transparent;
    cursor: pointer;
    text-transform: uppercase;
  }

  .kind-filters button:hover,
  .kind-filters button.active,
  .results-bar button:hover,
  .filtered-empty button:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .kind-filters button.active {
    background: var(--tint);
  }

  .results-bar {
    display: flex;
    justify-content: space-between;
    min-height: 46px;
    align-items: center;
    color: var(--muted);
    text-transform: uppercase;
  }

  .results-bar p {
    margin: 0;
  }

  .results-bar button {
    border: 0;
    padding-right: 0;
    text-transform: none;
  }

  .project-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px;
  }

  .project-card {
    display: flex;
    min-height: 330px;
    flex-direction: column;
    justify-content: space-between;
    padding: 19px 20px 17px;
    border: 1px solid var(--rule2);
    background: var(--panel);
    text-decoration: none;
    transition: border-color 160ms ease, transform 160ms ease, background 160ms ease;
  }

  .project-card:hover {
    border-color: var(--accent);
    background: var(--panel2);
    transform: translateY(-3px);
  }

  .card-topline,
  .card-meta {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }

  .card-index {
    color: var(--accent);
  }

  .card-arrow {
    color: var(--accent);
    font-size: 22px;
    line-height: 0.6;
  }

  .card-body {
    padding: 36px 0 50px;
  }

  .card-tags {
    display: flex;
    gap: 8px;
    min-height: 14px;
    color: var(--accent);
    text-transform: uppercase;
  }

  .card-tags span + span::before {
    content: '·';
    padding-right: 8px;
    color: var(--muted);
  }

  h3 {
    margin: 14px 0 9px;
    font: 400 36px/0.98 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.02em;
  }

  .card-body p {
    margin: 0;
    color: var(--soft);
    font-size: 17px;
    line-height: 1.4;
  }

  .card-meta {
    color: var(--muted);
    text-transform: uppercase;
    flex-wrap: wrap;
  }

  .card-meta span:first-child {
    color: var(--body);
  }

  .empty-state {
    display: grid;
    justify-items: center;
    gap: 12px;
    border: 1px dashed var(--rule2);
    padding: 60px 24px;
    color: var(--muted);
    text-align: center;
  }

  .empty-state p {
    margin: 0;
  }

  .empty-state a,
  .filtered-empty button {
    color: var(--accent);
    text-decoration: none;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  @media (max-width: 900px) {
    .directory-intro {
      grid-template-columns: 1fr;
      gap: 46px;
    }

    .directory-note p:last-child {
      max-width: 520px;
    }

    .project-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  @media (max-width: 620px) {
    .directory-intro {
      padding: 62px 0 72px;
    }

    h1 {
      font-size: clamp(60px, 17vw, 94px);
    }

    .directory-heading {
      align-items: start;
      flex-direction: column;
      gap: 14px;
    }

    .browse-controls {
      grid-template-columns: 1fr;
    }

    .kind-filters {
      grid-column: auto;
    }

    .project-grid {
      grid-template-columns: 1fr;
    }

    .project-card {
      min-height: 290px;
    }
  }
</style>
