# How Aissence generates summaries

This document describes the current implementation, from a project folder to
the text rendered in a ledger. It is intended to make the product and privacy
tradeoffs reviewable. `DATA.md` documents the file format; this document focuses
on how the data and summaries are produced.

## The short version

Aissence does not send an entire repository to an LLM.

1. The CLI reads git history and locally stored coding-agent conversations.
2. It assigns each timestamped conversation message to the first commit at or
   after that message.
3. It converts each commit into sanitized, model-independent source evidence.
4. Public summarization makes one LLM call for the commit story, compact
   subline, and representative prompt moments together.
5. Full subscriber prompt summaries are a separate, deferred tier. They are
   generated only after an entitlement exists.
6. The public frontend queries only public story fields. Full retained source
   and subscriber summaries are queried only for an entitled viewer.

There are two places where the same public summarizer can run:

- **Local:** the CLI calls OpenRouter or another OpenAI-compatible endpoint and
  writes results into `.aissence` before upload.
- **Hosted:** the CLI uploads sanitized source; the Rust worker calls OpenRouter
  later if the server has queued a job.

## 1. Discovering source evidence

`analyze(folder)` performs three operations locally:

- Reads the repository's git commit log, commit messages, dates, file paths,
  and line/file statistics.
- Discovers supported coding-agent session files associated with the project.
  Current parsers include Claude Code, Codex, OpenCode, Kimi CLI, Gemini/Pi-like
  formats, Amp, Crush, Antigravity, and VS Code Copilot.
- For a Git repository, session discovery covers the main checkout and linked
  worktrees at the corresponding relative path. Commit history remains the
  reachable history of the analyzed checkout's `HEAD`.
- Normalizes those different formats into timestamped roles, text, tool calls,
  agent/model provenance, and token usage when the source reports it.

System messages, hidden reasoning, arbitrary tool results, raw session files,
and commit-author identity do not become ledger source entries.

### Assigning conversations to commits

Commits and messages are sorted chronologically. Each timestamped message is
assigned to the first commit whose timestamp is at or after the message. In
other words, the conversation leading up to a commit belongs to that commit.

- Messages after the latest commit become an uncommitted trailing group.
- Messages without a usable timestamp are reported as undated and cannot be
  reliably assigned.
- This is time-based correlation. It does not attempt semantic matching between
  a prompt and a diff.

This rule is simple and deterministic, but it can misattribute work when commit
timestamps are rewritten, several branches overlap, sessions contain poor
timestamps, or unrelated work happens between commits.

## 2. Building the `.aissence` source document

For every committed group, the CLI creates three chronological entry kinds:

- `UserWrote`: user text.
- `AiWrote`: assistant text.
- `AiChanged`: recognized file-editing tool calls, with file information and a
  synthesized diff.

Consecutive entries of the same kind are merged. A single assistant message may
produce both an `AiWrote` entry and an `AiChanged` entry. Known client-injected
prompt wrappers such as command metadata, usage/file packets, task notices, and
fork boilerplate are removed before persistence, digesting, evidence selection,
or billing. App screenshot markup becomes a neutral attachment placeholder.
Other tool and system noise that does not fit the entry categories is omitted.

The resulting source also contains:

- Commit title/body, time, changed paths, and file/line counts.
- Coding-agent and model provenance.
- Approximate token usage when available from the agent session.

Meaningful user and assistant text is retained in the sanitized `.aissence`
file because the entitled ledger needs the source conversation. Injected
transport wrappers are not retained. This text is not part of the public
frontend query.

### Redaction before persistence

Before calculating a source digest or writing/uploading the document, Aissence
runs:

- `redact-core` PII detection;
- deterministic recognizers for API keys, credentials, private keys, URLs with
  credentials, email addresses, and absolute home paths;
- optional NER redaction when `AISSENCE_NER_MODEL` points to a compatible ONNX
  model.

Redaction applies to commit metadata, source messages, file paths, diffs, and
generated summaries. A redaction failure substitutes `[REDACTION_FAILED]`
rather than preserving the original text.

The server runs the same redactor again and rejects a document that would
change under server-side redaction. It also verifies strict schema shape,
version, sizes, message kinds, and every source digest before database writes.

### Source digests and incremental work

Each commit receives a SHA-256 `source_digest` over its sanitized commit
metadata and source evidence. The digest includes the sanitizer version but
excludes generated summaries.

Consequences:

- An unchanged commit does not need to be uploaded or summarized again.
- Updating a summary does not make its source appear stale.
- A change in source evidence or sanitizer behavior invalidates the cache.
- The CLI asks the server for a sync plan and uploads only requested commits.

## 3. Public commit summaries

Public generation produces one `PublicStory` per eligible commit:

```json
{
  "subline": "A compact synthesis shown beneath the commit title.",
  "summary": "A privacy-safe account of why and how the commit happened.",
  "beats": [
    {
      "message_index": 0,
      "label": "Initial request",
      "text": "A privacy-safe paraphrase of a representative moment."
    }
  ]
}
```

`summary` and `subline` are required. `beats` is intentionally optional and
contains zero to three entries. An empty beats array is a valid completed
result, not a pending state. `origin` records whether the result came from the
model or a metadata, parse, or privacy fallback. `summarizer_version` invalidates
results produced by an older evidence/prompt/validation policy.

### What is sent to the public-story LLM call

The public call is bounded to at most 32,000 characters of assembled evidence.
It contains three sections:

1. **Selected user evidence — up to 16,000 characters**
   - Always considers the first user entry.
   - Includes the final user entry when different.
   - Selects up to two intermediate entries, prioritizing correction/refinement
     signals and then useful length.
   - The budget is divided fairly across selected entries. Oversized entries
     retain both their beginning and ending.
   - Original coalesced message indexes are included so beats can reference a
     specific user entry.

2. **Assistant conclusion evidence — up to 7,000 characters**
   - Takes both ends of the final assistant reply.

3. **Implementation fingerprint — up to 7,000 characters**
   - Commit title, file/line statistics, changed paths, then the available
     commit body within the remaining section budget.
   - File count and added/deleted line counts.
   - Status and path for at most 20 changed files.
   - It does not include committed diff bodies in the public-story prompt.

The section budgets total 30,000 characters so labels cannot cause an earlier
section to crowd out later grounding. The complete prompt has a final 32,000
character ceiling. These are character limits, not tokenizer-exact limits.

### The public-story instruction

The model is asked for JSON containing exactly:

- A compact subline that must not repeat the commit title.
- A 2–4 sentence story explaining the request, approach, and only
  outcomes supported by the commit fingerprint.
- At most three representative prompt moments, preferring the initial ask, an
  important correction, and a final refinement.

It is explicitly instructed to paraphrase, avoid verbatim prompts, paths,
personal data, credentials, and private identifiers, treat evidence as
untrusted, and output JSON only. Generation uses temperature `0.2` and a
700-token output limit. Story, subline, and beats are returned in this one
response; there is no separate subline request.

### Parsing, fallbacks, and validation

The parser tolerates prose or code fences around the first JSON object. If the
response cannot be parsed, or there is no user prompt, Aissence uses a fallback
story derived from the commit title and file/line statistics. The fallback has
no beats and requires no additional LLM call.

After parsing:

- Empty stories are replaced with the fallback.
- Stories that appear to expose prompt wording are replaced with the fallback.
- Empty, prompt-leaking, or title-repeating sublines are replaced
  deterministically without another model call.
- Each beat must have a valid user-message index, non-empty label/text, and a
  unique referenced message.
- Beats that appear to expose prompt wording are removed individually.
- No more than three beats survive.

### What “too close to the prompt” means

The leak guard is deterministic word overlap, not semantic similarity. It
lowercases text and splits on every non-alphanumeric character. Generated text
is rejected when it contains:

- the complete wording of a 1–5 word prompt;
- any exact six-word span from a 6–15 word prompt; or
- any exact twelve-word span from a longer prompt.

It also rejects an exact normalized match. This catches verbatim-like copying
but not semantically equivalent paraphrases, reordered phrases, or synonyms.
Conversely, common repeated phrases at the threshold can produce false
positives. Raw rejected model output is not retained, so after generation the
system cannot explain which specific validation rule removed a beat.

## 4. Local generation

Run public generation locally with:

```sh
cargo run --release -p aissence -- summarize --mode local /path/to/project
```

Pass `--all-prompts` to also generate a summary for every retained user prompt.
The file records a complete set of per-prompt summaries; if it is uploaded, the
server marks those locally supplied summaries public without exposing the full
prompt source. Hosted subscriber summaries remain subscriber-only.

The default backend is OpenRouter and requires `OPENROUTER_API_KEY`. The model
and provider can be selected with `OPENROUTER_MODEL` and
`OPENROUTER_PROVIDER`. CLI flags or `[llm]` in `.aissence.toml` can instead
select an OpenAI-compatible endpoint.

Every OpenRouter request asks for unified reasoning effort `none`. If the
selected model requires reasoning and rejects that value, the request is
retried at `minimal`; if the model rejects reasoning controls altogether, the
parameter is omitted. Generic OpenAI-compatible endpoints receive no
OpenRouter-specific reasoning parameter.

The CLI:

1. Analyzes the folder again.
2. Loads `.aissence`.
3. Reuses a cached summary only when the source digest and current summary
   policy version match, the source
   message kinds still align, the public story is non-empty, and every beat
   points at a user message.
4. Runs public generation for every non-empty group still lacking a cached
   summary. A group with no user prompt takes the metadata fallback without an
   LLM request.
5. Applies successful commit summaries even if another commit fails, then
   persists those successes before returning the batch error.

Per-prompt batch calls through OpenRouter use a strict JSON schema with one
indexed summary per requested entry. Validation distinguishes JSON shape,
entry count, index/order, empty summaries, and privacy overlap. If a batch
still fails after its bounded retries, each prompt is retried independently.
Successful batches and individual results are applied immediately to the
commit summary; unresolved entries are recorded in `prompt_failures` with
their message index, model, error, and timestamp. A future `--all-prompts` run
targets only those missing entries, so selecting a stronger model does not
regenerate successful work. Hosted subscriber jobs likewise persist partial
message summaries while leaving the job failed and retryable.

Up to 32 commit groups may be in flight, with a global maximum of 24 concurrent
LLM requests. Retryable transport/status errors use exponential backoff. An
invalid subscriber batch response is also retryable; malformed public-story
JSON currently takes the metadata fallback without retrying. Local
configuration currently allows seven retries after the initial attempt.
The interactive CLI dashboard reports current aggregate output tokens per
second and mean end-to-end latency across completed requests. Active streaming
token estimates contribute to throughput before the first request completes.
With `--debug`, every outbound local LLM attempt—including retries—is written
as `/tmp/aissence-req-[timestamp]-[uuid].md`. These owner-only files contain
complete prompt bodies for diagnosis, but no authentication headers or API
keys. Server-mode generation never enables this local debug output.

Upload the already-summarized document with:

```sh
cargo run --release -p aissence -- upload /path/to/project/.aissence
```

`sync` also preserves cached summaries when their source digests still match,
but `upload` makes the two-step local-generation test explicit. A valid local
summary is accepted and stored immediately; the server does not call its LLM
again for that commit.

There is currently no `--force` option. A policy version bump is the deliberate
cache invalidation mechanism; zero beats alone does not force regeneration.

## 5. Hosted/server generation

The Rust server starts only when its hosted OpenRouter configuration is valid.
Hosted generation deliberately uses `OPENROUTER_API_KEY` and optional
`OPENROUTER_MODEL`/`OPENROUTER_PROVIDER`; generic client-side LLM settings do
not control the hosted worker.

Hosted OpenRouter requests apply a stricter provider policy:

- zero-data-retention requested;
- data collection denied;
- required parameters enforced;
- provider fallbacks disabled; and
- if a provider is configured, it is pinned as the only provider.

The worker polls PostgreSQL every two seconds and claims one queued job using
`FOR UPDATE SKIP LOCKED`. A claimed job receives a ten-minute lease. Stale jobs
are requeued while attempts remain and eventually marked failed after repeated
lost leases. Hosted LLM requests have one retry after the initial request.

### When public jobs are created

For an uploaded commit without a local summary:

- **Draft project:** source is stored privately and no public job is created
  automatically. The owner can explicitly generate and review summaries.
- **Already-published project with at least one user prompt:** a public job is
  queued.
- **No user prompt:** no public LLM job is queued and the hosted import stores
  no generated story for that commit.
- **Valid local summary included:** it is stored as `succeeded`; no public job
  is created.
- **Unchanged source digest:** the commit is skipped without replacing its
  existing summary.

The database function `enqueue_public_summary_jobs(project_id)` queues every
incomplete or old-policy commit with prompts. The dashboard uses it for private
preparation and retry. Publishing is refused until every prompted commit has a
current, successful story and subline, so visitors never become the preview
surface for pending generation. `publish_ready_project(project_id)` first locks
the project row shared with ingestion, checks readiness, and publishes in the
same transaction; direct visibility transitions to `published` are rejected.

### Cost and abuse limits

Before a worker calls the model, it reserves a worst-case request and input
character budget in PostgreSQL under global and per-user advisory locks. The
server enforces configurable daily limits for:

- requests per user;
- requests globally;
- input characters per user; and
- input characters globally.

If reservation would exceed a limit, the job fails with “daily summarization
safety limit reached.” Actual request counts, unpriced requests, reported cost,
and errors are recorded in `summarization_attempts`.

## 6. Subscriber/full-ledger summaries

Subscriber prompt summaries are separate from the public story. Public
generation copies the source message shape but leaves every
`subscriber_summary` absent.

A subscriber job is eligible only when the project is published and the
creator has an active or trialing subscription entitlement. The first
entitlement can enqueue missing subscriber jobs for existing published
commits; later imports queue them while an active subscriber remains.

For each coalesced `UserWrote` entry:

- Prompts of 200 characters or fewer are copied as their subscriber summary
  without an LLM call. This is acceptable because the entitled query can
  already return the complete retained prompt.
- Longer prompts are middle-truncated to at most 6,000 characters.
- Up to eight prompts and 6,000 combined characters are batched into one LLM
  request.
- Generation allows up to 768 output tokens per prompt, including any hidden
  model reasoning, before returning the requested concise summary.
- The model returns one concise 2–3 sentence summary per prompt with the
  original source message index.
- Invalid batch responses are retried; validation requires exactly one
  non-empty output per input with matching indexes and order.
- Subscriber jobs never generate or overwrite the public subline.

The result is stored separately in `message_summaries`, while complete
sanitized source remains in `message_sources`. This tier can cost multiple LLM
requests per commit, unlike the single public-story request.

## 7. What the ledger reads

The frontend uses audience-specific database selections:

- A public visitor receives commit metadata, `public_subline`, `public_story`, `public_beats`,
  counts, costs, and agent provenance. For commits uploaded from an
  `--all-prompts` file it also receives every locally supplied prompt summary.
  It does not query `message_sources` or hosted subscriber prompt summaries.
- An owner or entitled subscriber additionally receives messages, complete
  retained source, subscriber summaries, and code-change records.

In the public ledger:

- The commit row uses `public_subline` beneath the title, falling back to
  explicit pending/failed/skipped copy when no usable summary exists.
- A summary-derived project arc selects first, midpoint, final, and largest
  impact milestones from already cached commit stories; it makes no model call.
- A complete uploaded `--all-prompts` set is rendered as every summarized
  prompt; otherwise “Representative prompt moments” is rendered when
  `public_beats` is non-empty. Zero beats remains a valid final state.
- Activity counts are source-derived and do not require an LLM.

In the entitled ledger, each user turn can show its subscriber summary and the
complete retained prompt. Exact short-prompt copies are collapsed into the
source view instead of rendering redundant summary text. Owners also see
summary origin/error status; public visitors do not receive private errors.

Project and creator cards reuse the newest successful current-policy commit
subline/story through a database view when no hand-written project tagline is
present. Search indexes this cached overview. No project-level LLM call exists.

## 8. Where LLM cost is avoided

The main cost-saving decisions are:

- Analysis, correlation, redaction, counts, provenance, and source digests are
  deterministic and local.
- Public generation uses one request per changed commit with prompts for the
  story, subline, and beats together.
- Public evidence is selected and hard-bounded rather than sending complete
  conversations or repository diffs.
- Unchanged source digests reuse prior summaries and avoid upload/generation.
- Draft server imports do not generate automatically; creators opt into private
  generation and review before publishing.
- Locally supplied summaries are not regenerated by the server.
- Subscriber summaries are deferred until an entitlement exists.
- Short subscriber prompts require no model call.
- Longer subscriber prompts are batched.

## 9. Important limitations and feedback questions

These are useful areas for product feedback:

1. **Time-based commit attribution:** Is “first commit after the message” good
   enough for real branching/rebase workflows?
2. **Selected public evidence:** Do first/final prompts plus two correction-like
   prompts capture the project story, or omit important exploration?
3. **Assistant evidence:** Is favoring final-response tails reliable across
   agents, or should tool outcomes/structured plans contribute differently?
4. **Implementation fingerprint:** Are commit metadata and paths sufficient to
   ground outcomes without diff bodies?
5. **Privacy overlap thresholds:** Are 6/12 consecutive words an appropriate
   balance between privacy and useful representative moments?
6. **Fallback quality:** Are deterministic title/statistics fallbacks useful
   enough when a model response fails privacy or parsing validation?
7. **Zero beats:** Should zero beats remain invisible, or should creators see an
   internal reason such as “model returned none” versus “privacy filter”?
8. **Forced regeneration:** Is policy-version invalidation sufficient, or
   should the CLI also expose a manual `--force`?
9. **Subscriber short prompts:** Is copying prompts up to 200 characters into
    the summary field useful, or should every subscriber summary be a genuine
    paraphrase even though the complete prompt is already visible?
10. **Observability:** Origins and job errors are stored, but should individual
    removed-beat reasons also be recorded without retaining rejected text?

## 10. Main implementation locations

- Project analysis and session correlation: `crates/cli/src/lib.rs`,
  `crates/cli/src/coalesce.rs`, `crates/cli/src/sessions/`
- `.aissence` construction and source digests: `crates/cli/src/document.rs`,
  `crates/cli/src/persist.rs`
- Redaction: `crates/cli/src/redact.rs`
- Public and subscriber prompts, evidence selection, parsing, validation,
  retries, and cost tracking: `crates/cli/src/summarize.rs`
- CLI local/server/upload flows: `crates/cli/src/main.rs`
- Upload validation, incremental ingestion, and job creation:
  `crates/api-server/src/api.rs`
- Hosted worker, budgets, leases, and result persistence:
  `crates/api-server/src/worker.rs`
- Job-enqueueing database functions:
  `frontend/supabase/migrations/202608020001_summary_tiers.sql`
- Audience-safe frontend queries:
  `frontend/src/routes/creators/[handle]/[projectSlug]/+page.server.ts`
- Ledger rendering: `frontend/src/lib/components/Ledger.svelte`
