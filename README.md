# Aissence

Aissence correlates AI coding-agent conversations with Git commits and writes a
shareable, sanitized `.aissence` JSON document. Summaries can be generated
locally with OpenRouter or any standard OpenAI-compatible chat-completions API,
or by the authenticated Aissence server.

## Workspace

- `crates/cli`: shared library, CLI, GUI, and benchmark executable.
- `crates/api-server`: authenticated HTTP API, Postgres import queue, and
  summarization worker.

## CLI

```sh
# Create/update an unsummarized .aissence file.
cargo run -p aissence -- analyze /path/to/project

# Summarize directly with the user's OPENROUTER_API_KEY (the default).
cargo run -p aissence -- summarize --mode local /path/to/project

# Also summarize every user prompt; uploaded prompt summaries become public.
cargo run -p aissence -- summarize --mode local --all-prompts /path/to/project

# Or use a local/third-party OpenAI-compatible endpoint. The base URL includes
# /v1; Aissence appends /chat/completions. --api-key-env is optional.
cargo run -p aissence -- summarize --mode local /path/to/project \
  --api openai-compatible \
  --base-url http://127.0.0.1:11434/v1 \
  --model qwen2.5-coder:7b

# Incremental server summarization. The CLI offers browser login automatically
# when needed. A new folder also asks once for its project name and optional
# description, creates a private draft, and prints its creator preview URL.
cargo run -p aissence -- summarize --mode server /path/to/project

# Upload an existing locally summarized file without another model call.
cargo run -p aissence -- upload /path/to/project/.aissence
```

When analyzing a Git repository, Aissence also scans the corresponding folders
in its main and linked worktrees for agent sessions. Commit history remains
anchored to the `HEAD` of the folder being analyzed, so merged and rebased
worktree changes appear in the normal project history.

Credentials from browser login are stored in the operating system credential
store. Login uses a one-time device authorization page on the frontend origin,
so an existing dashboard session can approve the CLI without another sign-in.
If the session is missing, the normal magic-link flow returns to the approval
page. The frontend presents its Supabase access token to the API server-side;
the CLI receives only a short-lived Aissence access token and a rotating,
revocable refresh token.

Set `AISSENCE_NER_MODEL` to a compatible ONNX NER model. Without it, local and
server redaction still use `redact-core` pattern detection plus Aissence's
dedicated API-key, credential-assignment, and private-key detectors.

### Local LLM configuration

Local `summarize` accepts `--api`, `--base-url`, `--model`, `--api-key`,
`--api-key-env`, and the OpenRouter-only `--provider`. Passing a literal key on
the command line can expose it in shell history and process listings, so
`--api-key-env NAME` is preferred. Authentication is optional for compatible
local endpoints.

Pass `--debug` to a local summarize run to write every outbound LLM attempt to
`/tmp/aissence-req-[timestamp]-[uuid].md`. Files contain the complete serialized
request body, including prompt evidence, but never HTTP authorization headers
or API keys. On Unix they are created with owner-only permissions.

The same values can live under `[llm]` in `<project>/.aissence.toml`, or in a
file selected with `--llm-config PATH`:

```toml
[llm]
api = "openai-compatible"
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5-coder:7b"
fallback_model = "qwen2.5-coder:32b"
# api_key_env = "MY_LLM_API_KEY"
```

Use `api_key_env` rather than `api_key` in a file that might be committed.
Configuration precedence is CLI arguments, then environment variables, then
the TOML file, then built-in defaults. The generic environment names are
`AISSENCE_LLM_API`, `AISSENCE_LLM_BASE_URL`, `AISSENCE_LLM_MODEL`,
`AISSENCE_LLM_FALLBACK_MODEL`, `AISSENCE_LLM_API_KEY`,
`AISSENCE_LLM_API_KEY_ENV`, and `AISSENCE_LLM_PROVIDER`. Existing
`OPENROUTER_API_KEY`, `OPENROUTER_MODEL`, `OPENROUTER_FALLBACK_MODEL`, and
`OPENROUTER_PROVIDER` settings remain supported and OpenRouter remains the
zero-configuration backend. `--fallback-model` selects a better model for any
request or output-validation failure; it is unused when it matches the primary
model.

Compatible endpoints use `POST <base_url>/chat/completions` with standard
system/user messages, `temperature`, and `max_tokens`. OpenRouter-specific
provider routing, ZDR policy fields, streaming usage, and cost metadata are
only enabled for OpenRouter. OpenRouter requests also set unified reasoning
effort to `none`; if a model requires reasoning, Aissence retries with
`minimal`, then omits the reasoning parameter if the model rejects it entirely.
Other endpoints use the more widely implemented
non-streaming response path; token usage is recorded when returned, but cost is
reported as unpriced unless the endpoint supplies OpenRouter-compatible cost
metadata.

OpenRouter batch prompt summaries use a strict JSON schema. An invalid request
or response first switches to the configured fallback model. Invalid batches
then fall back to individual prompt requests, successful partial results remain
in the `.aissence` cache, and unresolved indexes retain model/error metadata for
a later retry with the same or a different model.

Public summaries use a two-stage pipeline in both the CLI and server: every
user prompt is first converted to a privacy-safe bounded summary, then the
commit narrative is synthesized from those summaries plus bounded assistant
and git evidence. Its requested range scales from 60–90 words for compact
evidence to 120–150 for substantial commits, with a stable 50–165-word
validation envelope across all models. Normal/hosted generation keeps the intermediate
summaries private and ephemeral; local `--all-prompts` stores the same results
without making additional requests.

## Server

The development defaults target the local Supabase instance:

```text
DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:55422/postgres
AISSENCE_BIND=127.0.0.1:8787
```

The server also reads:

- `OPENROUTER_API_KEY` and optional `OPENROUTER_MODEL`,
  `OPENROUTER_FALLBACK_MODEL`, and `OPENROUTER_PROVIDER`.
- `SUPABASE_JWT_SECRET` for validating browser login approval.
- `AISSENCE_TOKEN_SECRET`, issuer, audience, public API URL, frontend site URL,
  and token lifetimes. Set `AISSENCE_SITE_URL` to the frontend origin used for
  CLI authorization and creator preview links.
- Configurable upload, entry, concurrency, and daily request safety limits.

Run it with:

```sh
cargo run -p aissence-server
```

For the complete local workflow, configure `OPENROUTER_API_KEY` in the root
`.env` file (or export it) and run this from the repository root:

```sh
scripts/dev.sh
```

This starts local Supabase, the Rust API, and the frontend against the same
Postgres database. Use `scripts/dev.sh --open` to also open the frontend and
Mailpit. Then exercise project creation and incremental uploads from another
terminal:

```sh
cargo run -p aissence -- sync /path/to/project
```

### Fast local summary previews

For quick iteration on the summarization prompts and ledger rendering, run:

```sh
scripts/preview-summary.sh /path/to/project openai/gpt-5.4

# Also generate a summary for every user prompt.
scripts/preview-summary.sh --all-prompts /path/to/project openai/gpt-5.4
```

Add a stronger fallback model for failed requests or invalid summaries with:

```sh
scripts/preview-summary.sh --fallback-model openai/gpt-5.4 \
  /path/to/project inclusionai/ling-2.6-flash
```

The script removes the target repository's existing `.aissence` cache, runs
local summarization with the selected OpenRouter model, starts `scripts/dev.sh`
in the background when the frontend is not already available, and opens an
in-memory preview in the default browser. Re-running it reuses the development
server. The preview upload requires no login, but exists only in Vite
development mode, accepts only a loopback host, and disappears when the server
restarts (or after 24 hours). The normal authenticated upload path is unchanged.

The startup migration is idempotent and creates the import queue, usage ledger,
and hashed device/refresh-token tables. Production must use HTTPS and unique,
high-entropy values for both JWT secrets. Aissence stores hashes—not plaintext—
for all one-time and refresh tokens.

## Incremental synchronization

The CLI sends a compact manifest of `(commit_hash, source_digest)` pairs. The
server returns only missing or changed hashes, and the CLI uploads a valid
`.aissence` document containing that delta. A full manifest handles rebases and
older commits whose agent sessions were backfilled; server-only commits are not
deleted automatically.

The protocol is split into explicit steps:

- `POST /v1/auth/device`, browser approval, token polling, refresh, and logout.
- `PUT /v1/projects/resolve` maps the file's stable `source_id` to an
  owner-scoped server project.
- `POST /v1/projects/{id}/sync-plan` compares the full hash/digest manifest.
- `POST /v1/projects/{id}/imports` accepts only the requested valid v2 delta.
- `GET /v1/imports/{id}` and `/result` expose completion and mergeable results.

An uploaded commit that already has a valid local summary is stored directly
and never queued for OpenRouter. This preserves the manual local-summarize and
upload workflow.

Malformed or shape-invalid JSON is rejected before an import row or OpenRouter
request is created. Every upload is size/count bounded, source digests are
verified, and documents that still change under server-side redaction are
rejected.

Before an upstream model call, the worker reserves a conservative upstream-call
call and input-character ceiling (including the single allowed retry) in
Postgres under global and per-user advisory locks. Daily limits therefore hold
across processes, and a crashed/expired worker remains charged for the reserved
ceiling rather than replaying a possibly paid request. The hosted OpenRouter
client uses a server-selected model, allows at most one retry, disables provider
fallbacks, and requires no-data-collection/ZDR routing. Users cannot supply a
model, prompt, provider, token limit, or arbitrary inference request.
Production should also configure a hard spending limit on the dedicated
OpenRouter key as a final independent circuit breaker.

## Model benchmarks

```sh
cargo run -p aissence --bin aissence-benchmark -- \
  --model inclusionai/ling-2.6-flash \
  --source-folder /path/to/project
```

The benchmark remains OpenRouter-specific. Set `OPENROUTER_API_KEY` first. It
uses the same bounded, one-request public-story flow as normal generation.
