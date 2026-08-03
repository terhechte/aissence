<script lang="ts">
  import { goto } from '$app/navigation';
  import { repositoryFields } from '$lib/aissence/project';
  import { parseAissence, AissenceValidationError } from '$lib/aissence/schema';
  import { getBrowserSupabase } from '$lib/supabase/client';
  import { slugify } from '$lib/utils';

  let projectFile = $state<File | null>(null);
  let fileSummary = $state<{ commits: number; warnings: number } | null>(null);
  let screenshots = $state<Array<{ file: File; alt: string; caption: string }>>([]);
  let name = $state('');
  let slug = $state('');
  let slugTouched = $state(false);
  let tagline = $state('');
  let description = $state('');
  let productUrl = $state('');
  let repositoryLabel = $state('');
  let repositoryUrl = $state('');
  let busy = $state(false);
  let status = $state('');
  let error = $state('');
  let issues = $state<string[]>([]);

  function setName(value: string) {
    name = value;
    if (!slugTouched) slug = slugify(value);
  }

  function prefillProjectFields(project: { name?: string; repository?: string } | undefined) {
    if (!project) return;
    if (project.name) setName(project.name);
    if (project.repository) {
      const repository = repositoryFields(project.repository);
      repositoryLabel = repository.label;
      repositoryUrl = repository.url;
    }
  }

  async function inspectFile(file: File | undefined) {
    error = '';
    issues = [];
    fileSummary = null;
    if (!file) {
      projectFile = null;
      return;
    }
    if (file.size > 25 * 1024 * 1024) {
      error = 'The project file must be no larger than 25 MB.';
      return;
    }
    try {
      const parsed = parseAissence(await file.text());
      projectFile = file;
      prefillProjectFields(parsed.file.project);
      fileSummary = {
        commits: parsed.snapshot.commits.length,
        warnings: parsed.warnings.length
      };
    } catch (caught) {
      projectFile = null;
      if (caught instanceof AissenceValidationError) {
        error = caught.message;
        issues = caught.issues;
      } else {
        error = 'The selected file could not be read.';
      }
    }
  }

  function addScreenshots(files: FileList | null) {
    if (!files) return;
    const next = [...files].slice(0, 8 - screenshots.length).map((file) => ({
      file,
      alt: '',
      caption: ''
    }));
    screenshots = [...screenshots, ...next];
  }

  async function responseJson(response: Response) {
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error ?? 'The request failed.');
    return body;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = '';
    issues = [];
    if (!projectFile || !fileSummary) {
      error = 'Choose a valid .aissence file.';
      return;
    }
    const form = new FormData(event.currentTarget as HTMLFormElement);
    busy = true;
    try {
      status = 'Creating private draft…';
      const project = await responseJson(
        await fetch('/api/projects', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(Object.fromEntries(form))
        })
      );
      const projectId = project.id as string;
      const supabase = getBrowserSupabase();
      if (!supabase) throw new Error('Supabase is not configured.');

      status = 'Uploading project history…';
      const importTicket = await responseJson(
        await fetch(`/api/projects/${projectId}/imports/prepare`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ size: projectFile.size })
        })
      );
      const { error: importUploadError } = await supabase.storage
        .from('aissence-staging')
        .uploadToSignedUrl(importTicket.path, importTicket.token, projectFile, {
          contentType: 'application/json'
        });
      if (importUploadError) throw importUploadError;

      const imported = await responseJson(
        await fetch(`/api/projects/${projectId}/imports/complete`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ path: importTicket.path })
        })
      );
      issues = imported.warnings ?? [];

      if (screenshots.length) {
        status = 'Uploading screenshots…';
        const screenshotTicket = await responseJson(
          await fetch(`/api/projects/${projectId}/screenshots/prepare`, {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({
              files: screenshots.map(({ file }) => ({ type: file.type, size: file.size }))
            })
          })
        );
        for (let index = 0; index < screenshots.length; index += 1) {
          const shot = screenshots[index];
          const ticket = screenshotTicket.uploads[index];
          const { error: uploadError } = await supabase.storage
            .from('project-media')
            .uploadToSignedUrl(ticket.path, ticket.token, shot.file, { contentType: shot.file.type });
          if (uploadError) throw uploadError;
        }

        await responseJson(
          await fetch(`/api/projects/${projectId}/screenshots/complete`, {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({
              screenshots: screenshots.map((shot, index) => ({
                storage_path: screenshotTicket.uploads[index].path,
                alt_text: shot.alt,
                caption: shot.caption,
                sort_order: index
              }))
            })
          })
        );
      }

      status = 'Draft ready.';
      await goto(`/dashboard/projects/${projectId}`);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'The draft could not be created.';
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>New project — aissence</title></svelte:head>

<div class="page-shell new-project">
  <header>
    <p class="eyebrow">New project</p>
    <h1>Start with the conversation.</h1>
    <p>The file stays private while you add context and review the resulting ledger.</p>
  </header>

  <form onsubmit={submit}>
    <section>
      <div class="step-head"><span>01</span><div><h2>Project history</h2><p>Drop the JSON file generated in your repository.</p></div></div>
      <label
        class:valid={fileSummary}
        class="drop-zone"
        ondragover={(event) => event.preventDefault()}
        ondrop={(event) => {
          event.preventDefault();
          inspectFile(event.dataTransfer?.files[0]);
        }}
      >
        <input
          type="file"
          accept=".aissence,application/json"
          onchange={(event) => inspectFile(event.currentTarget.files?.[0])}
        />
        {#if fileSummary}
          <strong>{projectFile?.name}</strong>
          <span>{fileSummary.commits} commits · {fileSummary.warnings} legacy warnings</span>
        {:else}
          <strong>Drop .aissence here</strong>
          <span>or choose a file · up to 25 MB</span>
        {/if}
      </label>
    </section>

    <section>
      <div class="step-head"><span>02</span><div><h2>Project context</h2><p>Give readers the shape of what was built.</p></div></div>
      <div class="form-grid">
        <div class="field">
          <label for="name">Project name</label>
          <input id="name" name="name" required maxlength="120" value={name} oninput={(e) => setName(e.currentTarget.value)} />
        </div>
        <div class="field">
          <label for="slug">Public slug</label>
          <input id="slug" name="slug" required value={slug} oninput={(e) => { slugTouched = true; slug = slugify(e.currentTarget.value); }} />
        </div>
        <div class="field wide">
          <label for="tagline">Tagline <span>(optional)</span></label>
          <input id="tagline" name="tagline" maxlength="240" bind:value={tagline} />
        </div>
        <div class="field wide">
          <label for="description">Description <span>(optional)</span></label>
          <textarea id="description" name="description" maxlength="3000" bind:value={description}></textarea>
        </div>
        <div class="field">
          <label for="product_url">Product URL <span>(optional)</span></label>
          <input id="product_url" name="product_url" type="url" pattern="https://.*" placeholder="https://" bind:value={productUrl} />
        </div>
        <div class="field">
          <label for="repository_label">Repository label <span>(optional)</span></label>
          <input id="repository_label" name="repository_label" placeholder="owner/repository" bind:value={repositoryLabel} />
        </div>
        <div class="field wide">
          <label for="repository_url">Repository URL <span>(optional)</span></label>
          <input id="repository_url" name="repository_url" type="url" pattern="https://.*" placeholder="https://" bind:value={repositoryUrl} />
        </div>
      </div>
    </section>

    <section>
      <div class="step-head"><span>03</span><div><h2>What it became</h2><p>Add up to eight optional screenshots. Alt text is optional.</p></div></div>
      <label class="screenshot-picker">
        <input
          type="file"
          accept="image/jpeg,image/png,image/webp"
          multiple
          onchange={(event) => addScreenshots(event.currentTarget.files)}
        />
        Add screenshots
      </label>
      {#if screenshots.length}
        <div class="screenshots">
          {#each screenshots as shot, index}
            <article>
              <img src={URL.createObjectURL(shot.file)} alt="" />
              <div class="field">
                <label for={`alt-${index}`}>Alt text <span>(optional)</span></label>
                <input id={`alt-${index}`} bind:value={shot.alt} />
              </div>
              <div class="field">
                <label for={`caption-${index}`}>Caption <span>(optional)</span></label>
                <input id={`caption-${index}`} bind:value={shot.caption} />
              </div>
              <button type="button" onclick={() => (screenshots = screenshots.filter((_, i) => i !== index))}>Remove</button>
            </article>
          {/each}
        </div>
      {/if}
    </section>

    {#if error}<p class="error">{error}</p>{/if}
    {#if issues.length && !error}
      <details class="notice"><summary>{issues.length} compatibility warnings</summary><ul>{#each issues as issue}<li>{issue}</li>{/each}</ul></details>
    {/if}
    <div class="submit-row">
      <p>{status || 'Nothing is public until you publish the draft.'}</p>
      <button class="button" disabled={busy}>{busy ? 'Building draft…' : 'Create private draft'}</button>
    </div>
  </form>
</div>

<style>
  .new-project {
    padding-block: 58px 120px;
  }
  header {
    max-width: 760px;
    margin-bottom: 70px;
  }
  header h1 {
    margin: 15px 0 20px;
    font: 400 clamp(60px, 9vw, 104px) / 0.9 'Instrument Serif', Georgia, serif;
  }
  header > p:last-child {
    max-width: 570px;
    color: var(--body);
    font-size: 20px;
    line-height: 1.5;
  }
  form {
    display: grid;
    gap: 70px;
  }
  section {
    border-top: 1px solid var(--ink);
    padding-top: 22px;
  }
  .step-head {
    display: grid;
    grid-template-columns: 60px 1fr;
    margin-bottom: 28px;
  }
  .step-head > span {
    color: var(--accent);
    font: 500 11px/1 'JetBrains Mono', monospace;
  }
  .step-head h2 {
    margin: 0;
    font: 400 35px/1 'Instrument Serif', Georgia, serif;
  }
  .step-head p {
    color: var(--muted);
    margin: 8px 0 0;
  }
  .drop-zone {
    min-height: 190px;
    border: 1px dashed var(--rule2);
    display: grid;
    place-content: center;
    gap: 8px;
    text-align: center;
    cursor: pointer;
    background: var(--panel);
  }
  .drop-zone.valid {
    border-style: solid;
    border-color: var(--accent);
  }
  .drop-zone input,
  .screenshot-picker input {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    opacity: 0;
  }
  .drop-zone strong {
    font: 400 29px/1.2 'Instrument Serif', Georgia, serif;
  }
  .drop-zone span {
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .field span {
    color: var(--faint);
  }
  .screenshot-picker {
    display: inline-flex;
    padding: 12px 18px;
    border: 1px solid var(--accent);
    color: var(--accent);
    font: 500 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    cursor: pointer;
  }
  .screenshots {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 24px;
    margin-top: 25px;
  }
  .screenshots article {
    border: 1px solid var(--rule2);
    padding: 14px;
    display: grid;
    gap: 14px;
  }
  .screenshots img {
    display: block;
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: cover;
    background: var(--panel2);
  }
  .screenshots button {
    justify-self: start;
    border: 0;
    padding: 0;
    background: none;
    color: var(--danger);
    cursor: pointer;
    text-decoration: underline;
  }
  .submit-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 25px;
    border-top: 1px solid var(--ink);
    padding-top: 20px;
  }
  .submit-row p {
    color: var(--muted);
  }
  @media (max-width: 720px) {
    .screenshots {
      grid-template-columns: 1fr;
    }
    .submit-row {
      align-items: stretch;
      flex-direction: column;
    }
  }
</style>
