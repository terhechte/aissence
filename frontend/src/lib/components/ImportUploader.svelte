<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { parseAissence, AissenceValidationError } from '$lib/aissence/schema';
  import { getBrowserSupabase } from '$lib/supabase/client';

  let { projectId } = $props<{ projectId: string }>();
  let file = $state<File | null>(null);
  let commitCount = $state<number | null>(null);
  let warnings = $state<string[]>([]);
  let error = $state('');
  let busy = $state(false);
  let status = $state('');

  async function inspect(selected: File | undefined) {
    error = '';
    warnings = [];
    commitCount = null;
    if (!selected) return;
    try {
      const parsed = parseAissence(await selected.text());
      file = selected;
      commitCount = parsed.snapshot.commits.length;
      warnings = parsed.warnings;
    } catch (caught) {
      file = null;
      error =
        caught instanceof AissenceValidationError
          ? caught.issues.join(' ')
          : 'The selected file could not be read.';
    }
  }

  async function asJson(response: Response) {
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error ?? 'The request failed.');
    return body;
  }

  async function upload() {
    if (!file) return;
    busy = true;
    error = '';
    try {
      status = 'Uploading…';
      const ticket = await asJson(
        await fetch(`/api/projects/${projectId}/imports/prepare`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ size: file.size })
        })
      );
      const supabase = getBrowserSupabase();
      if (!supabase) throw new Error('Supabase is not configured.');
      const { error: uploadError } = await supabase.storage
        .from('aissence-staging')
        .uploadToSignedUrl(ticket.path, ticket.token, file, { contentType: 'application/json' });
      if (uploadError) throw uploadError;
      status = 'Applying snapshot…';
      const result = await asJson(
        await fetch(`/api/projects/${projectId}/imports/complete`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ path: ticket.path })
        })
      );
      warnings = result.warnings ?? [];
      status = result.noop ? 'This exact snapshot was already imported.' : `${result.commitCount} commits imported.`;
      file = null;
      commitCount = null;
      await invalidateAll();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Import failed.';
      status = '';
    } finally {
      busy = false;
    }
  }
</script>

<div class="uploader">
  <label class="drop" ondragover={(e) => e.preventDefault()} ondrop={(e) => { e.preventDefault(); inspect(e.dataTransfer?.files[0]); }}>
    <input type="file" accept=".aissence,application/json" onchange={(e) => inspect(e.currentTarget.files?.[0])} />
    <strong>{file ? file.name : 'Choose a new .aissence snapshot'}</strong>
    <span>{commitCount === null ? 'Up to 25 MB' : `${commitCount} commits ready`}</span>
  </label>
  {#if file}<button class="button" type="button" onclick={upload} disabled={busy}>{busy ? status : 'Replace snapshot'}</button>{/if}
  {#if status && !busy}<p class="notice">{status}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
  {#if warnings.length}<details><summary>{warnings.length} compatibility warnings</summary><ul>{#each warnings as warning}<li>{warning}</li>{/each}</ul></details>{/if}
</div>

<style>
  .uploader {
    display: grid;
    gap: 14px;
  }
  .drop {
    min-height: 125px;
    border: 1px dashed var(--rule2);
    background: var(--panel);
    display: grid;
    place-content: center;
    gap: 7px;
    text-align: center;
    cursor: pointer;
  }
  .drop input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
  }
  .drop strong {
    font: 400 23px/1.2 'Instrument Serif', Georgia, serif;
  }
  .drop span,
  details {
    color: var(--muted);
    font: 400 10px/1.5 'JetBrains Mono', monospace;
  }
  .button {
    justify-self: start;
  }
</style>
