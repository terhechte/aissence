<script lang="ts">
  import { micromark } from 'micromark';
  import { filterPromptNoise } from '$lib/prompt';

  let { content }: { content: string } = $props();
  const rendered = $derived(micromark(filterPromptNoise(content)));
</script>

<div class="prompt-markdown">{@html rendered}</div>

<style>
  .prompt-markdown {
    color: var(--body3);
    font: 400 16px/1.68 Newsreader, Georgia, serif;
  }
  .prompt-markdown :global(> :first-child) {
    margin-top: 0;
  }
  .prompt-markdown :global(> :last-child) {
    margin-bottom: 0;
  }
  .prompt-markdown :global(h1),
  .prompt-markdown :global(h2),
  .prompt-markdown :global(h3),
  .prompt-markdown :global(h4) {
    margin: 1.2em 0 0.45em;
    color: var(--ink);
    font-family: 'Instrument Serif', Georgia, serif;
    font-weight: 400;
    line-height: 1.15;
  }
  .prompt-markdown :global(h1) {
    font-size: 29px;
  }
  .prompt-markdown :global(h2) {
    font-size: 25px;
  }
  .prompt-markdown :global(h3),
  .prompt-markdown :global(h4) {
    font-size: 21px;
  }
  .prompt-markdown :global(p),
  .prompt-markdown :global(ul),
  .prompt-markdown :global(ol),
  .prompt-markdown :global(blockquote),
  .prompt-markdown :global(pre) {
    margin: 0.75em 0;
  }
  .prompt-markdown :global(ul),
  .prompt-markdown :global(ol) {
    padding-left: 1.4em;
  }
  .prompt-markdown :global(blockquote) {
    border-left: 3px solid var(--accent);
    padding-left: 1em;
    color: var(--soft);
  }
  .prompt-markdown :global(code) {
    padding: 0.12em 0.3em;
    background: var(--panel2);
    font: 400 0.82em/1.45 'JetBrains Mono', monospace;
  }
  .prompt-markdown :global(pre) {
    max-width: 100%;
    overflow: auto;
    padding: 15px;
    border: 1px solid var(--rule2);
    background: var(--paper);
  }
  .prompt-markdown :global(pre code) {
    padding: 0;
    background: none;
    font-size: 12px;
  }
  .prompt-markdown :global(a) {
    color: var(--accent);
    overflow-wrap: anywhere;
  }
</style>
