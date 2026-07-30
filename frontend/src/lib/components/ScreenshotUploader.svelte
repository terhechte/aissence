<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { getBrowserSupabase } from '$lib/supabase/client';

  let { projectId, startOrder = 0 } = $props<{ projectId: string; startOrder?: number }>();
  let items = $state<Array<{ file: File; alt: string; caption: string }>>([]);
  let busy = $state(false);
  let error = $state('');

  function choose(files: FileList | null) {
    if (!files) return;
    items = [...files].slice(0, 8 - startOrder).map((file) => ({ file, alt: '', caption: '' }));
  }

  async function asJson(response: Response) {
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error ?? 'Upload failed.');
    return body;
  }

  async function upload() {
    if (!items.length || items.some((item) => !item.alt.trim())) {
      error = 'Describe every screenshot before uploading.';
      return;
    }
    busy = true;
    error = '';
    try {
      const ticket = await asJson(
        await fetch(`/api/projects/${projectId}/screenshots/prepare`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ files: items.map(({ file }) => ({ type: file.type, size: file.size })) })
        })
      );
      const supabase = getBrowserSupabase();
      if (!supabase) throw new Error('Supabase is not configured.');
      for (let index = 0; index < items.length; index += 1) {
        const upload = ticket.uploads[index];
        const item = items[index];
        const { error: uploadError } = await supabase.storage
          .from('project-media')
          .uploadToSignedUrl(upload.path, upload.token, item.file, { contentType: item.file.type });
        if (uploadError) throw uploadError;
      }
      await asJson(
        await fetch(`/api/projects/${projectId}/screenshots/complete`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            screenshots: items.map((item, index) => ({
              storage_path: ticket.uploads[index].path,
              alt_text: item.alt,
              caption: item.caption,
              sort_order: startOrder + index
            }))
          })
        })
      );
      items = [];
      await invalidateAll();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : 'Screenshots could not be uploaded.';
    } finally {
      busy = false;
    }
  }
</script>

{#if startOrder < 8}
  <div class="screenshot-uploader">
    <label class="button secondary">
      <input type="file" accept="image/jpeg,image/png,image/webp" multiple onchange={(e) => choose(e.currentTarget.files)} />
      Add screenshots
    </label>
    {#if items.length}
      <div class="items">
        {#each items as item, index}
          <article>
            <img src={URL.createObjectURL(item.file)} alt="" />
            <div class="field"><label for={`new-alt-${index}`}>Alt text</label><input id={`new-alt-${index}`} bind:value={item.alt} /></div>
            <div class="field"><label for={`new-caption-${index}`}>Caption</label><input id={`new-caption-${index}`} bind:value={item.caption} /></div>
          </article>
        {/each}
      </div>
      <button class="button" type="button" onclick={upload} disabled={busy}>{busy ? 'Uploading…' : 'Upload images'}</button>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </div>
{/if}

<style>
  .screenshot-uploader {
    display: grid;
    gap: 18px;
  }
  label.button input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
  }
  label.button {
    justify-self: start;
    cursor: pointer;
  }
  .items {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 18px;
  }
  article {
    padding: 12px;
    border: 1px solid var(--rule2);
    display: grid;
    gap: 12px;
  }
  img {
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: cover;
  }
  @media (max-width: 650px) {
    .items {
      grid-template-columns: 1fr;
    }
  }
</style>
