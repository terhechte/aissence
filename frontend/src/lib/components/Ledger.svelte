<script lang="ts">
  import PromptMarkdown from '$lib/components/PromptMarkdown.svelte';
  import { abridgeTranscript, groupTranscript } from '$lib/ledger';
  import { formatCount, formatDate } from '$lib/utils';

  let {
    profile,
    project,
    commits,
    screenshots,
    isEntitled = false
  } = $props<{
    profile: any;
    project: any;
    commits: any[];
    screenshots: any[];
    isEntitled?: boolean;
  }>();

  let openCommit = $state<string | null>(null);
  let activeShot = $state(0);
  let modal = $state(false);
  let chartMode = $state<'tokens' | 'lines'>('tokens');

  const totals = $derived.by(() => {
    const sum = (key: string) =>
      commits.reduce((total: number, commit: any) => total + Number(commit[key] ?? 0), 0);
    const tokenInput = sum('token_input');
    const tokenOutput = sum('token_output');
    const inputCost = sum('input_cost_usd');
    const outputCost = sum('output_cost_usd');
    const potentialCost = sum('potential_cost_usd');
    const pricedCommits = commits.filter((commit: any) => commit.potential_cost_usd !== null).length;
    const unpricedCommits = commits.filter(
      (commit: any) => commit.token_input !== null && commit.potential_cost_usd === null
    ).length;
    const agentNames = new Set(
      commits.flatMap((commit: any) => commit.commit_agents.map((agent: any) => agent.provider))
    );
    const prompts = commits.reduce(
      (total: number, commit: any) =>
        total + commit.messages.filter((message: any) => message.kind === 'UserWrote').length,
      0
    );
    const dated = commits.filter((commit: any) => commit.committed_at);
    const first = dated[0]?.committed_at;
    const last = dated.at(-1)?.committed_at;
    const span =
      first && last ? Math.round((Date.parse(last) - Date.parse(first)) / 86_400_000) + 1 : null;
    return {
      tokenInput,
      tokenOutput,
      inputCost,
      outputCost,
      potentialCost,
      pricedCommits,
      unpricedCommits,
      linesAdded: sum('lines_added'),
      linesDeleted: sum('lines_deleted'),
      files: sum('files_changed'),
      agents: agentNames,
      prompts,
      first,
      last,
      span
    };
  });

  const maxChartValue = $derived.by(() => {
    const keys = chartMode === 'tokens' ? ['token_input', 'token_output'] : ['lines_added', 'lines_deleted'];
    return Math.max(
      1,
      ...commits.flatMap((commit: any) => keys.map((key) => Number(commit[key] ?? 0)))
    );
  });

  function monthKey(commit: any) {
    if (!commit.committed_at) return 'Undated';
    const date = new Date(commit.committed_at);
    return `${date.getUTCFullYear()}-${date.getUTCMonth()}`;
  }

  function monthLabel(commit: any) {
    if (!commit.committed_at) return 'Undated';
    return new Intl.DateTimeFormat('en', { timeZone: 'UTC', month: 'long', year: 'numeric' }).format(
      new Date(commit.committed_at)
    );
  }

  function sourceContent(message: any): string | null {
    if (Array.isArray(message.message_sources)) return message.message_sources[0]?.content ?? null;
    return message.message_sources?.content ?? null;
  }

  function messageTime(message: any) {
    if (!message.occurred_at) return '—';
    return new Date(message.occurred_at).toISOString().slice(11, 16);
  }

  function kindLabel(kind: string) {
    return kind === 'UserWrote' ? 'asked' : kind === 'AiWrote' ? 'replied' : 'changed';
  }

  function messagePreview(message: any) {
    return sourceContent(message) || message.preview || 'Text unavailable.';
  }

  function changeDescription(message: any) {
    const files = [...new Set((message.changes ?? []).map((change: any) => change.file_path))];
    if (files.length) {
      const shown = files.slice(0, 4).join(', ');
      const remaining = files.length - 4;
      return `Changed ${shown}${remaining > 0 ? ` and ${remaining} more` : ''}.`;
    }
    const count = Number(message.changed_file_count ?? 0);
    return count
      ? `Changed ${count} ${count === 1 ? 'file' : 'files'}.`
      : 'Recorded a code change.';
  }

  function promptCount(commit: any) {
    return commit.messages.filter((message: any) => message.kind === 'UserWrote').length;
  }

  function activityText(replyCount: number, changedFileCount: number) {
    return `The agent replied ${replyCount} ${replyCount === 1 ? 'time' : 'times'} and changed ${changedFileCount} ${changedFileCount === 1 ? 'file' : 'files'}.`;
  }

  function transcriptItems(commit: any) {
    const turns = groupTranscript(commit.messages);
    if (isEntitled) return turns.map((turn) => ({ kind: 'turn' as const, turn }));
    return abridgeTranscript(turns, sourceContent, commit.commit_hash);
  }

  function hiddenActivityText(promptCount: number, replyCount: number, changedFileCount: number) {
    return `${promptCount} hidden user ${promptCount === 1 ? 'prompt' : 'prompts'} and ${activityText(replyCount, changedFileCount)}`;
  }

  function formatUsd(value: number | string | null | undefined) {
    if (value === null || value === undefined || !Number.isFinite(Number(value))) return 'Unavailable';
    const amount = Number(value);
    if (amount > 0 && amount < 0.01) return '<$0.01';
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: 'USD',
      minimumFractionDigits: 2,
      maximumFractionDigits: amount < 1 ? 4 : 2
    }).format(amount);
  }

  function totalCostLabel() {
    if (!totals.pricedCommits) return 'Unavailable';
    return `${formatUsd(totals.potentialCost)}${totals.unpricedCommits ? '+' : ''}`;
  }

  function toggleCommit(id: string) {
    openCommit = openCommit === id ? null : id;
  }
</script>

<div class="ledger page-shell">
  <div class="ledger-top">
    <span>{project.repository_label ? `Ledger — ${project.repository_label}` : 'Project ledger'}</span>
    <a href={`/creators/${profile.handle}`}>{profile.display_name}</a>
  </div>

  <section class="project-intro">
    <div>
      <p class="eyebrow">A project, reconstructed from its commits</p>
      <h1>{project.name}</h1>
      <p class="tagline">{project.tagline}</p>
      <p class="description">{project.description}</p>
      <div class="project-links">
        {#if project.product_url}<a href={project.product_url} target="_blank" rel="noreferrer">Visit project ↗</a>{/if}
        {#if project.repository_url}<a href={project.repository_url} target="_blank" rel="noreferrer">Repository ↗</a>{/if}
        {#if totals.first && totals.last}<span>{formatDate(totals.first)} – {formatDate(totals.last)}</span>{/if}
      </div>
    </div>

    <div class="gallery">
      <div class="gallery-head"><span>What it became</span><span>{String(activeShot + 1).padStart(2, '0')} / {String(screenshots.length).padStart(2, '0')}</span></div>
      {#if screenshots.length}
        <button class="hero-shot" onclick={() => (modal = true)} aria-label="Enlarge active screenshot">
          <img src={screenshots[activeShot].url} alt={screenshots[activeShot].alt_text || ''} />
        </button>
        {#if screenshots[activeShot].caption || screenshots[activeShot].alt_text}
          <p>{screenshots[activeShot].caption || screenshots[activeShot].alt_text}</p>
        {/if}
        <div class="thumbnails">
          {#each screenshots as shot, index}
            <button
              class:active={activeShot === index}
              onclick={() => (activeShot = index)}
              aria-label={`Show screenshot ${index + 1}`}
            ><img src={shot.url} alt="" /></button>
          {/each}
        </div>
      {:else}
        <div class="empty-state">No screenshots available.</div>
      {/if}
    </div>
  </section>

  <section class="account">
    <div class="chart">
      <div class="chart-head">
        <p>{chartMode === 'tokens' ? 'Tokens read above the line, written below' : 'Lines added above the line, removed below'}</p>
        <div><button class:active={chartMode === 'tokens'} onclick={() => (chartMode = 'tokens')}>tokens</button><button class:active={chartMode === 'lines'} onclick={() => (chartMode = 'lines')}>lines</button></div>
      </div>
      <div class="bars">
        {#each commits as commit, index}
          {@const up = Number(commit[chartMode === 'tokens' ? 'token_input' : 'lines_added'] ?? 0)}
          {@const down = Number(commit[chartMode === 'tokens' ? 'token_output' : 'lines_deleted'] ?? 0)}
          <button
            class:dimmed={openCommit && openCommit !== commit.id}
            onclick={() => toggleCommit(commit.id)}
            aria-label={`Open ${commit.title || commit.commit_hash.slice(0, 10)}`}
          >
            <span class="up" style={`height:${Math.max(2, Math.round((up / maxChartValue) * 100))}px`}></span>
            <i></i>
            <span class="down" style={`height:${Math.max(2, Math.round((down / maxChartValue) * 34))}px`}></span>
            <small>{index % 4 === 2 ? formatDate(commit.committed_at, { month: 'short', day: 'numeric', year: undefined }) : ''}</small>
          </button>
        {/each}
      </div>
    </div>
    <aside>
      <p class="label">The whole account</p>
      <dl>
        <div><dt>Tokens read by model</dt><dd><strong>{formatCount(totals.tokenInput)}</strong><small>{totals.pricedCommits ? formatUsd(totals.inputCost) : 'Cost unavailable'}</small></dd></div>
        <div><dt>Tokens written back</dt><dd><strong>{formatCount(totals.tokenOutput)}</strong><small>{totals.pricedCommits ? formatUsd(totals.outputCost) : 'Cost unavailable'}</small></dd></div>
        <div><dt>Estimated API costs</dt><dd><strong>{totalCostLabel()}</strong><small>USD estimate</small></dd></div>
        <div><dt>Commits from a conversation</dt><dd><strong>{commits.length}</strong></dd></div>
        <div><dt>Total user prompts</dt><dd><strong>{totals.prompts}</strong></dd></div>
        <div><dt>Days represented</dt><dd><strong>{totals.span ?? 'unknown'}</strong></dd></div>
        <div><dt>Lines added / removed</dt><dd><strong>+{formatCount(totals.linesAdded)} / −{formatCount(totals.linesDeleted)}</strong></dd></div>
        <div><dt>Agents at the keyboard</dt><dd><strong>{totals.agents.size}</strong></dd></div>
      </dl>
      <p class="account-note">
        {[...totals.agents].join(', ') || 'Agent provenance unavailable'}.
        {#if totals.unpricedCommits} Cost totals exclude {totals.unpricedCommits} unpriced {totals.unpricedCommits === 1 ? 'commit' : 'commits'}.{/if}
      </p>
    </aside>
  </section>

  <section class="entries">
    <div class="entries-head">
      <span>The entries</span>
      <span>Select a line to read what was asked, decided and built</span>
    </div>
    {#if commits.length}
      {#each commits as commit, index}
        {#if index === 0 || monthKey(commit) !== monthKey(commits[index - 1])}
          <div class="month"><h2>{monthLabel(commit)}</h2><i></i><span>{commits.filter((item: any) => monthKey(item) === monthKey(commit)).length} entries</span></div>
        {/if}
        <article class:open={openCommit === commit.id}>
          <button class="commit-row" onclick={() => toggleCommit(commit.id)} aria-expanded={openCommit === commit.id}>
            <div class="commit-date">
              <span class="commit-date-dash" aria-hidden="true">—</span>
              <span class="commit-date-values">
                <time>{formatDate(commit.committed_at, { month: 'short', day: 'numeric', year: undefined })}</time>
                <small>{String(index + 1).padStart(2, '0')}</small>
              </span>
            </div>
            <div><h3>{commit.title || `Commit ${commit.commit_hash.slice(0, 10)}`}</h3><p>{commit.user_requested}</p><span>{commit.commit_agents.map((agent: any) => `${agent.provider}${agent.model ? ` · ${agent.model}` : ''}`).join(' / ') || 'Agent unknown'}</span></div>
            <div class="token-cell">
              <strong>{formatCount(commit.token_input)} in <em>{formatUsd(commit.input_cost_usd)}</em></strong>
              <strong>{formatCount(commit.token_output)} out <em>{formatUsd(commit.output_cost_usd)}</em></strong>
              <small>{formatUsd(commit.potential_cost_usd)} estimated</small>
            </div>
            <div class="line-cell">
              {#if commit.lines_added !== null}<span>+{commit.lines_added}</span><span>−{commit.lines_deleted}</span><small>{commit.files_changed} files · {promptCount(commit)} prompts</small>{:else}<small>{promptCount(commit)} prompts · stats unavailable</small>{/if}
            </div>
            <b>{openCommit === commit.id ? '↑' : '↓'}</b>
          </button>

          {#if openCommit === commit.id}
            <div class="commit-detail">
              <div class="commit-meta"><span>{formatDate(commit.committed_at)}</span><span>{commit.commit_hash.slice(0, 10)}</span></div>
              <div>
                <div class="summary-grid">
                  <div><span>Decided</span><p>{commit.ai_answered || 'No response summary available.'}</p></div>
                  <div><span>Built</span><p>{commit.ai_implemented || 'No implementation summary available.'}</p></div>
                </div>

                {#if commit.messages.length}
                  <div class="transcript">
                    <h4>{isEntitled ? 'Transcript' : 'Transcript, abridged'}</h4>
                    {#each transcriptItems(commit) as item}
                      {#if item.kind === 'hidden'}
                        <div class="message activity-summary hidden-summary">
                          <span>hidden</span>
                          <time></time>
                          <div><p>{hiddenActivityText(item.promptCount, item.replyCount, item.changedFileCount)}</p></div>
                        </div>
                      {:else}
                        {@const turn = item.turn}
                      {#if turn.asked}
                        <div class="message asked-message">
                          <span>asked</span>
                          <time>{messageTime(turn.asked)}</time>
                          <div>
                            <p>{turn.asked.summary}</p>
                            {#if isEntitled}
                              {@const content = sourceContent(turn.asked)}
                              {#if content}
                                <details class="full-prompt">
                                  <summary>Read full prompt</summary>
                                  <PromptMarkdown content={content} />
                                </details>
                              {:else}
                                <p class="source-unavailable">Full prompt unavailable in this source file.</p>
                              {/if}
                            {:else}
                              <div class="locked">
                                <span>Full prompt for subscribers</span>
                                <a href={`/creators/${profile.handle}/subscribe`}>Unlock prompts →</a>
                              </div>
                            {/if}
                          </div>
                        </div>
                      {/if}

                      {#if isEntitled}
                        {#if turn.activity.length}
                          <details class="agent-activity">
                            <summary>
                              <span>{activityText(turn.replyCount, turn.changedFileCount)}</span>
                              <b>Show replied / changed</b>
                            </summary>
                            <div class="activity-rows">
                              {#each turn.activity as message}
                                {@const content = messagePreview(message)}
                                <div class="message">
                                  <span class:ai={message.kind === 'AiWrote'}>{kindLabel(message.kind)}</span>
                                  <time>{messageTime(message)}</time>
                                  <div>
                                    {#if message.kind === 'AiChanged'}
                                      <p>{changeDescription(message)}</p>
                                    {:else}
                                      <PromptMarkdown content={content} />
                                    {/if}
                                  </div>
                                </div>
                              {/each}
                            </div>
                          </details>
                        {/if}
                      {:else if turn.asked}
                        <div class="message activity-summary">
                          <span>agent</span>
                          <time></time>
                          <div><p>{activityText(turn.replyCount, turn.changedFileCount)}</p></div>
                        </div>
                      {/if}
                      {/if}
                    {/each}
                  </div>
                {/if}

                <div class="facts">
                  <div><span>tokens in</span><strong>{formatCount(commit.token_input)} · {formatUsd(commit.input_cost_usd)}</strong></div>
                  <div><span>tokens out</span><strong>{formatCount(commit.token_output)} · {formatUsd(commit.output_cost_usd)}</strong></div>
                  <div><span>estimated API cost</span><strong>{formatUsd(commit.potential_cost_usd)}</strong></div>
                  <div><span>prompts written</span><strong>{promptCount(commit)}</strong></div>
                  <div><span>lines</span><strong>{commit.lines_added === null ? 'unknown' : `+${commit.lines_added} / −${commit.lines_deleted}`}</strong></div>
                  <div><span>files changed</span><strong>{commit.files_changed ?? 'unknown'}</strong></div>
                </div>
              </div>
            </div>
          {/if}
        </article>
      {/each}
    {:else}
      <div class="empty-state">This project does not contain any imported commits.</div>
    {/if}
  </section>

  <footer>
    <p>Every line above came out of a conversation. Aissence keeps the two together, so a repository can still explain itself a year from now.</p>
    <a href={`/creators/${profile.handle}`}>More from {profile.display_name} →</a>
  </footer>
</div>

{#if modal && screenshots.length}
  <button class="modal" onclick={() => (modal = false)} aria-label="Close enlarged screenshot">
    <img src={screenshots[activeShot].url} alt={screenshots[activeShot].alt_text || ''} />
    {#if screenshots[activeShot].caption || screenshots[activeShot].alt_text}
      <span>{screenshots[activeShot].caption || screenshots[activeShot].alt_text}</span>
    {/if}
  </button>
{/if}

<style>
  .ledger {
    padding-bottom: 115px;
  }
  .ledger-top {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 24px 0 18px;
    border-bottom: 1px solid var(--ink);
    color: var(--muted);
    font: 400 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.13em;
  }
  .project-intro {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(280px, 1fr);
    gap: 64px;
    padding-top: 55px;
    align-items: start;
  }
  h1 {
    font: 400 clamp(76px, 10vw, 112px) / 0.88 'Instrument Serif', Georgia, serif;
    letter-spacing: -0.025em;
    margin: 20px 0 18px;
  }
  .tagline {
    font: italic 300 25px/1.35 Newsreader, serif;
    color: var(--body2);
    max-width: 16em;
    margin: 0 0 25px;
  }
  .description {
    font-size: 17px;
    line-height: 1.62;
    color: var(--body);
    max-width: 36em;
  }
  .project-links {
    display: flex;
    gap: 25px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 28px;
  }
  .project-links a {
    text-decoration-color: var(--accent);
  }
  .project-links span {
    color: var(--muted);
    font: 400 10px/1 'JetBrains Mono', monospace;
  }
  .gallery {
    border-top: 1px solid var(--ink);
    padding-top: 13px;
  }
  .gallery-head {
    display: flex;
    justify-content: space-between;
    margin-bottom: 14px;
    color: var(--muted);
    font: 400 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  .hero-shot {
    display: block;
    width: 100%;
    border: 1px solid var(--rule2);
    padding: 0;
    background: var(--panel);
    cursor: zoom-in;
  }
  .hero-shot img {
    display: block;
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: cover;
  }
  .gallery > p {
    margin: 8px 0 12px;
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  .thumbnails {
    display: flex;
    gap: 8px;
  }
  .thumbnails button {
    flex: 1;
    border: 1px solid var(--rule2);
    padding: 2px;
    background: transparent;
    cursor: pointer;
    opacity: 0.58;
  }
  .thumbnails button.active {
    border-color: var(--accent);
    opacity: 1;
  }
  .thumbnails img {
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: cover;
    display: block;
  }
  .account {
    display: grid;
    grid-template-columns: minmax(0, 1.7fr) minmax(260px, 0.8fr);
    gap: 60px;
    margin-top: 70px;
    border-top: 1px solid var(--ink);
    padding-top: 14px;
  }
  .chart-head {
    display: flex;
    justify-content: space-between;
    gap: 20px;
  }
  .chart-head p {
    margin: 0;
    color: var(--muted);
    font: 400 10px/1.4 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  .chart-head button {
    border: 0;
    background: transparent;
    color: var(--muted);
    font: 400 9px/1 'JetBrains Mono', monospace;
    cursor: pointer;
  }
  .chart-head button.active {
    color: var(--accent);
    text-decoration: underline;
  }
  .bars {
    height: 178px;
    margin-top: 25px;
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--rule);
  }
  .bars button {
    align-self: stretch;
    flex: 1;
    display: grid;
    grid-template-rows: 104px 1px 36px 22px;
    align-items: end;
    border: 0;
    padding: 0 1px;
    background: transparent;
    cursor: pointer;
    transition: opacity 0.2s;
  }
  .bars button.dimmed {
    opacity: 0.28;
  }
  .bars span {
    display: block;
    width: 72%;
    max-width: 16px;
    justify-self: center;
    background: var(--ink);
  }
  .bars span.down {
    align-self: start;
    background: var(--accent);
  }
  .bars i {
    width: 100%;
    height: 1px;
    background: var(--rule2);
  }
  .bars small {
    color: var(--faint);
    font: 400 8px/1 'JetBrains Mono', monospace;
    white-space: nowrap;
  }
  .account aside {
    border-top: 1px solid var(--ink);
    padding-top: 13px;
  }
  .account aside > p {
    margin-top: 0;
  }
  .account dl {
    margin: 18px 0 0;
  }
  .account dl div {
    display: flex;
    justify-content: space-between;
    gap: 20px;
    padding: 9px 0;
    border-bottom: 1px solid var(--rule);
  }
  .account dt {
    color: var(--soft);
  }
  .account dd {
    display: grid;
    justify-items: end;
    gap: 4px;
    margin: 0;
    text-align: right;
  }
  .account dd strong {
    font: 500 13px/1 'JetBrains Mono', monospace;
  }
  .account dd small {
    color: var(--faint);
    font: 400 8px/1.2 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .account .account-note {
    margin-top: 15px;
    color: var(--muted);
    font-style: italic;
    line-height: 1.45;
  }
  .entries {
    margin-top: 78px;
  }
  .entries-head {
    display: flex;
    justify-content: space-between;
    gap: 25px;
    border-bottom: 1px solid var(--ink);
    padding-bottom: 12px;
    color: var(--muted);
    font: 400 10px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  .entries-head span:last-child {
    font-family: Newsreader, serif;
    font-style: italic;
    text-transform: none;
    letter-spacing: 0;
    font-size: 13px;
  }
  .month {
    display: flex;
    align-items: baseline;
    gap: 18px;
    padding: 40px 0 9px;
  }
  .month h2 {
    margin: 0;
    font: 400 34px/1 'Instrument Serif', Georgia, serif;
  }
  .month i {
    flex: 1;
    height: 1px;
    background: var(--rule2);
  }
  .month span {
    color: var(--muted);
    font: 400 9px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.1em;
  }
  article.open {
    background: var(--panel);
  }
  .commit-row {
    display: grid;
    grid-template-columns: 74px minmax(0, 1fr) 158px 108px 22px;
    gap: 22px;
    width: 100%;
    align-items: baseline;
    padding: 17px 8px 17px 0;
    border: 0;
    border-bottom: 1px solid var(--rule);
    background: transparent;
    color: var(--ink);
    text-align: left;
    cursor: pointer;
  }
  .commit-row:hover {
    background: var(--panel2);
  }
  .commit-row time,
  .commit-row small,
  .commit-row span,
  .commit-row strong {
    font-family: 'JetBrains Mono', monospace;
  }
  .commit-row > div:first-child {
    display: grid;
    gap: 4px;
  }
  .commit-row .commit-date {
    grid-template-columns: 12px minmax(0, 1fr);
    column-gap: 8px;
  }
  .commit-date-dash {
    color: var(--muted);
    transition: opacity 0.25s ease;
  }
  article.open .commit-date-dash {
    opacity: 0;
  }
  .commit-date-values {
    display: grid;
    gap: 4px;
  }
  .commit-row > div:first-child time {
    font-size: 11px;
  }
  .commit-row small {
    color: var(--faint);
    font-size: 9px;
  }
  .commit-row h3 {
    margin: 0;
    font: 400 22px/1.25 'Instrument Serif', Georgia, serif;
  }
  .commit-row > div:nth-child(2) p {
    display: -webkit-box;
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    max-width: 48em;
    margin: 4px 0 8px;
    color: var(--soft);
    font-size: 14px;
    line-height: 1.45;
  }
  article.open .commit-row > div:nth-child(2) p {
    display: block;
    overflow: visible;
    max-width: 54em;
    font-size: 16px;
    line-height: 1.55;
  }
  .commit-row > div:nth-child(2) span {
    color: var(--accent);
    font-size: 9px;
    text-transform: uppercase;
  }
  .token-cell,
  .line-cell {
    display: grid;
    gap: 5px;
  }
  .token-cell strong,
  .line-cell span {
    font-size: 10px;
    font-weight: 500;
  }
  .token-cell strong {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .token-cell em {
    color: var(--faint);
    font-size: 8px;
    font-style: normal;
    font-weight: 400;
  }
  .token-cell strong:nth-child(2) {
    color: var(--accent);
  }
  .line-cell {
    text-align: right;
  }
  .commit-row > b {
    color: var(--accent);
    font: 400 14px/1 'JetBrains Mono', monospace;
  }
  .commit-detail {
    display: grid;
    grid-template-columns: 74px minmax(0, 1fr);
    gap: 22px;
    padding: 32px 8px 38px 0;
    border-bottom: 1px solid var(--rule);
  }
  .commit-meta {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-left: 20px;
    color: var(--muted);
    font: 400 9px/1.4 'JetBrains Mono', monospace;
  }
  .summary-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 28px;
    padding-top: 20px;
    border-top: 1px solid var(--rule2);
  }
  .summary-grid span,
  .transcript h4,
  .facts span {
    color: var(--accent);
    font: 400 9px/1 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  .summary-grid p {
    color: var(--body);
    line-height: 1.55;
  }
  .transcript {
    margin-top: 28px;
    border-top: 1px solid var(--rule2);
    padding-top: 19px;
  }
  .transcript h4 {
    color: var(--muted);
  }
  .message {
    display: grid;
    grid-template-columns: 70px 46px 1fr;
    gap: 14px;
    align-items: baseline;
    padding: 8px 0;
  }
  .message > span,
  .message > time {
    color: var(--soft);
    font: 400 9px/1.4 'JetBrains Mono', monospace;
    text-transform: uppercase;
  }
  .message > span.ai {
    color: var(--accent);
  }
  .message > time {
    color: var(--faint);
  }
  .message p {
    margin: 0;
    color: var(--body);
    line-height: 1.5;
  }
  .full-prompt,
  .locked,
  .source-unavailable {
    margin-top: 10px;
  }
  .full-prompt summary {
    color: var(--accent);
    cursor: pointer;
    font: 500 10px/1.4 'JetBrains Mono', monospace;
  }
  .full-prompt :global(.prompt-markdown) {
    margin-top: 12px;
    padding: 18px 20px;
    border: 1px solid var(--rule2);
    background: var(--paper);
  }
  .locked {
    display: flex;
    justify-content: space-between;
    gap: 15px;
    padding: 10px 12px;
    background: var(--tint);
    color: var(--muted);
    font: 400 9px/1.3 'JetBrains Mono', monospace;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .locked a {
    color: var(--accent);
  }
  .source-unavailable {
    color: var(--muted) !important;
    font-style: italic;
  }
  .agent-activity {
    margin: 3px 0 13px 130px;
    border: 1px solid var(--rule2);
    background: var(--paper);
  }
  .agent-activity > summary {
    display: flex;
    justify-content: space-between;
    gap: 20px;
    padding: 11px 13px;
    color: var(--soft);
    cursor: pointer;
    font: 400 10px/1.4 'JetBrains Mono', monospace;
    list-style: none;
  }
  .agent-activity > summary::-webkit-details-marker {
    display: none;
  }
  .agent-activity > summary b {
    flex: none;
    color: var(--accent);
    font-size: 9px;
    font-weight: 500;
    text-transform: uppercase;
  }
  .agent-activity[open] > summary {
    border-bottom: 1px solid var(--rule2);
  }
  .activity-rows {
    padding: 4px 12px 8px;
  }
  .activity-summary {
    margin-left: 130px;
    border-left: 2px solid var(--rule2);
    padding: 10px 12px;
    color: var(--muted);
  }
  .activity-summary > span {
    color: var(--faint);
  }
  .activity-summary p {
    color: var(--muted);
    font-style: italic;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 32px;
    margin-top: 27px;
    border-top: 1px solid var(--rule2);
    padding-top: 17px;
  }
  .facts div {
    display: grid;
    gap: 6px;
  }
  .facts span {
    color: var(--faint);
  }
  .facts strong {
    font: 500 12px/1 'JetBrains Mono', monospace;
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 30px;
    margin-top: 85px;
    border-top: 1px solid var(--ink);
    padding-top: 18px;
  }
  footer p {
    max-width: 34em;
    color: var(--body);
    font: italic 300 19px/1.5 Newsreader, serif;
  }
  .modal {
    position: fixed;
    inset: 0;
    z-index: 100;
    width: 100%;
    border: 0;
    padding: 5vh 5vw;
    background: rgb(20 19 14 / 92%);
    display: grid;
    place-items: center;
    gap: 15px;
    color: #ede7db;
    cursor: zoom-out;
  }
  .modal img {
    max-width: 90vw;
    max-height: 80vh;
    object-fit: contain;
  }
  .modal span {
    font: 400 10px/1.4 'JetBrains Mono', monospace;
  }
  @media (max-width: 850px) {
    .project-intro,
    .account {
      grid-template-columns: 1fr;
    }
    .commit-row {
      grid-template-columns: 64px 1fr 20px;
    }
    .token-cell,
    .line-cell {
      display: none;
    }
    .summary-grid {
      grid-template-columns: 1fr;
    }
  }
  @media (max-width: 600px) {
    .entries-head span:last-child {
      display: none;
    }
    .commit-detail {
      grid-template-columns: 1fr;
    }
    .commit-meta {
      flex-direction: row;
      padding-left: 20px;
    }
    .message {
      grid-template-columns: 60px 1fr;
    }
    .message > time {
      display: none;
    }
    .agent-activity,
    .activity-summary {
      margin-left: 0;
    }
    footer {
      align-items: flex-start;
      flex-direction: column;
    }
  }
</style>
