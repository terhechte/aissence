<script lang="ts">
  import { enhance } from '$app/forms';
  import { invalidateAll } from '$app/navigation';
  import ImportUploader from '$lib/components/ImportUploader.svelte';
  import ScreenshotUploader from '$lib/components/ScreenshotUploader.svelte';
  import { formatDate } from '$lib/utils';
  import { onMount } from 'svelte';

  let { data, form } = $props();
  const previewUrl = $derived(`/creators/${data.project.profiles.handle}/${data.project.slug}`);
  const hasActiveSummaryJobs = $derived(
    data.summaryJobs.some((job: any) => ['queued', 'processing'].includes(job.status))
  );

  onMount(() => {
    let refreshing = false;
    const timer = window.setInterval(async () => {
      if (!hasActiveSummaryJobs || refreshing || document.visibilityState !== 'visible') return;
      refreshing = true;
      try {
        await invalidateAll();
      } finally {
        refreshing = false;
      }
    }, 2_000);
    return () => window.clearInterval(timer);
  });
</script>

<svelte:head><title>{data.project.name} — dashboard</title></svelte:head>

<div class="page-shell project-admin">
  <header>
    <div>
      <p class="eyebrow">{data.project.visibility === 'published' ? 'Published ledger' : 'Private draft'}</p>
      <h1>{data.project.name}</h1>
      <p>{data.commitCount} commits · last updated {formatDate(data.project.updated_at)}</p>
    </div>
    <div class="header-actions">
      <a class="button secondary" href={previewUrl}>Preview ledger</a>
      {#if data.project.visibility === 'published'}
        <form method="POST" action="?/unpublish" use:enhance><button class="button secondary">Unpublish</button></form>
      {:else}
        <form method="POST" action="?/publish" use:enhance>
          <button
            class="button"
            disabled={data.summaryReadiness.ready < data.summaryReadiness.prompted}
            title={data.summaryReadiness.ready < data.summaryReadiness.prompted ? 'Generate and review summaries first' : 'Publish ledger'}
          >Publish</button>
        </form>
      {/if}
      <form method="POST" action="?/deleteProject" onsubmit={(event) => { if (!confirm('Permanently delete this project?')) event.preventDefault(); }}>
        <button class="button danger">Delete project</button>
      </form>
    </div>
  </header>
  {#if form?.error}<p class="error">{form.error}</p>{/if}
  {#if form?.saved}<p class="notice">Project details saved.</p>{/if}
  {#if form?.summariesQueued !== undefined}<p class="notice">Queued {form.summariesQueued} private summary {form.summariesQueued === 1 ? 'job' : 'jobs'}.</p>{/if}

  <section>
    <div class="section-head"><span>01</span><div><h2>Project details</h2><p>The public framing around the imported history.</p></div></div>
    <form class="metadata form-grid" method="POST" action="?/updateMetadata" use:enhance>
      <div class="field"><label for="name">Project name</label><input id="name" name="name" value={data.project.name} required /></div>
      <div class="field"><label for="slug">Public slug</label><input id="slug" name="slug" value={data.project.slug} required /></div>
      <div class="field wide"><label for="tagline">Tagline (optional)</label><input id="tagline" name="tagline" maxlength="240" value={data.project.tagline} /></div>
      <div class="field wide"><label for="description">Description (optional)</label><textarea id="description" name="description" maxlength="3000">{data.project.description}</textarea></div>
      <div class="field"><label for="product_url">Product URL <span>(optional)</span></label><input id="product_url" name="product_url" type="url" value={data.project.product_url ?? ''} pattern="https://.*" /></div>
      <div class="field"><label for="repository_label">Repository label <span>(optional)</span></label><input id="repository_label" name="repository_label" value={data.project.repository_label ?? ''} /></div>
      <div class="field wide"><label for="repository_url">Repository URL</label><input id="repository_url" name="repository_url" type="url" value={data.project.repository_url ?? ''} /></div>
      <div class="field wide">
        <label for="comment_visibility">Comments</label>
        <select id="comment_visibility" name="comment_visibility" value={data.project.comment_visibility ?? 'subscribers'}>
          <option value="subscribers">Subscribers only</option>
          <option value="public">Everyone</option>
        </select>
        <small>When comments are enabled, choose who can read and post on this ledger.</small>
      </div>
      <button class="button">Save details</button>
    </form>
  </section>

  <section>
    <div class="section-head"><span>02</span><div><h2>Project history</h2><p>Uploads store source privately. Generate and review every public story before publishing.</p></div></div>
    <ImportUploader projectId={data.project.id} />
    {#if data.imports.length}
      <div class="import-log">
        {#each data.imports as item}
          <div><span>{item.status}</span><span>{item.commit_count ?? '—'} commits</span><time>{formatDate(item.created_at)}</time></div>
        {/each}
      </div>
    {/if}
    <div class="summary-job-note">
      <strong>Public-story readiness</strong>
      <span>{data.summaryReadiness.ready} / {data.summaryReadiness.prompted} ready</span>
      <span>{data.summaryReadiness.pending} pending</span>
      <span>{data.summaryReadiness.failed} failed</span>
      {#if data.summaryReadiness.fallbacks}<span>{data.summaryReadiness.fallbacks} fallback</span>{/if}
      {#if data.summaryReadiness.ready < data.summaryReadiness.prompted && data.summaryReadiness.pending === 0}
        <form method="POST" action="?/prepareSummaries" use:enhance><button>Generate private summaries</button></form>
      {/if}
      {#if data.summaryReadiness.ready === data.summaryReadiness.prompted && data.summaryReadiness.prompted > 0}
        <a href={previewUrl}>Review summaries →</a>
      {/if}
    </div>
    {#if data.summaryJobs.some((job: any) => job.status === 'failed')}
      <details class="summary-errors">
        <summary>Summary errors</summary>
        <ul>
          {#each data.summaryJobs.filter((job: any) => job.status === 'failed') as job}
            <li><strong>{job.kind}</strong>: {job.error_detail || 'Unknown worker failure'}</li>
          {/each}
        </ul>
      </details>
    {/if}
    {#if data.summaryJobs.length}
      <div class="summary-job-note secondary-status">
        <span>{data.summaryJobs.filter((job: any) => ['queued', 'processing'].includes(job.status)).length} worker jobs pending</span>
        <span>{data.summaryJobs.filter((job: any) => job.status === 'succeeded').length} complete</span>
        {#if data.summaryJobs.some((job: any) => job.kind === 'public' && job.status === 'failed')}
          <form method="POST" action="?/retrySummaries" use:enhance><button>Retry failed public stories</button></form>
        {/if}
      </div>
    {/if}
  </section>

  <section>
    <div class="section-head"><span>03</span><div><h2>Screenshots</h2><p>These become the “What it became” gallery.</p></div></div>
    {#if data.screenshots.length}
      <div class="shots">
        {#each data.screenshots as shot, index}
          <article>
            <img src={shot.url} alt={shot.alt_text || ''} />
            <div>
              <strong>{shot.caption || shot.alt_text || `Screenshot ${index + 1}`}</strong>
              {#if shot.alt_text}<span>{shot.alt_text}</span>{/if}
            </div>
            <div class="shot-actions">
              <form method="POST" action="?/moveScreenshot" use:enhance>
                <input type="hidden" name="screenshot_id" value={shot.id} />
                <button name="direction" value="-1" disabled={index === 0} aria-label="Move screenshot earlier">←</button>
                <button name="direction" value="1" disabled={index === data.screenshots.length - 1} aria-label="Move screenshot later">→</button>
              </form>
              <form method="POST" action="?/removeScreenshot" use:enhance>
                <input type="hidden" name="screenshot_id" value={shot.id} />
                <button class="remove" aria-label="Remove screenshot">Remove</button>
              </form>
            </div>
          </article>
        {/each}
      </div>
    {/if}
    <ScreenshotUploader projectId={data.project.id} startOrder={data.screenshots.length} />
  </section>

</div>

<style>
  .project-admin {
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
    font: 400 clamp(62px, 9vw, 104px) / 0.9 'Instrument Serif', Georgia, serif;
    margin: 16px 0;
  }
  header p:last-child {
    color: var(--muted);
  }
  .header-actions {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  section {
    border-top: 1px solid var(--ink);
    padding-top: 22px;
    margin-top: 66px;
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
  .metadata {
    max-width: 900px;
  }
  .metadata > button {
    justify-self: start;
  }
  .import-log {
    margin-top: 25px;
    border-top: 1px solid var(--rule);
  }
  .import-log > div {
    display: grid;
    grid-template-columns: 100px 1fr 130px;
    border-bottom: 1px solid var(--rule);
    padding: 11px 0;
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .summary-job-note {
    display: flex;
    gap: 18px;
    margin-top: 18px;
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .summary-job-note strong { color: var(--body); }
  .summary-job-note button,
  .summary-job-note a { color: var(--accent); text-decoration: underline; }
  .secondary-status { margin-top: 8px; }
  .summary-errors { margin-top: 12px; color: var(--danger); font-size: 13px; }
  .summary-errors ul { margin-bottom: 0; }
  .shots {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 24px;
    margin-bottom: 25px;
  }
  .shots article {
    border: 1px solid var(--rule2);
    padding: 13px;
  }
  .shots img {
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: cover;
    display: block;
  }
  .shots article > div:nth-child(2) {
    display: grid;
    gap: 4px;
    margin: 12px 0;
  }
  .shots strong {
    font-weight: 400;
  }
  .shots span {
    color: var(--muted);
    font-size: 13px;
  }
  .shot-actions {
    display: flex;
    justify-content: space-between;
  }
  .shot-actions form {
    display: flex;
    gap: 6px;
  }
  .shot-actions button {
    border: 1px solid var(--rule2);
    background: transparent;
    cursor: pointer;
  }
  .shot-actions .remove {
    border: 0;
    color: var(--danger);
    text-decoration: underline;
  }
  @media (max-width: 760px) {
    header {
      align-items: flex-start;
      flex-direction: column;
    }
    .shots {
      grid-template-columns: 1fr;
    }
  }
</style>
