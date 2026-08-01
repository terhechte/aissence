# Aissence

Aissence correlates AI coding-agent conversations with Git commits and writes a
shareable, sanitized `.aissence` JSON document. Summaries can be generated
locally with the user's OpenRouter key or by the authenticated Aissence server.

## Workspace

- `crates/aissence`: shared library, CLI, GUI, and benchmark executable.
- `crates/aissence-server`: authenticated HTTP API, Postgres import queue, and
  summarization worker.

## CLI

```sh
# Create/update an unsummarized .aissence file.
cargo run -p aissence -- analyze /path/to/project

# Summarize directly with the user's OPENROUTER_API_KEY.
cargo run -p aissence -- summarize --mode local /path/to/project

# Browser login, then incremental server summarization.
cargo run -p aissence -- login
cargo run -p aissence -- summarize --mode server /path/to/project

# Upload an existing locally summarized file without another model call.
cargo run -p aissence -- upload /path/to/project/.aissence
```

Credentials from browser login are stored in the operating system credential
store. Login uses a one-time device authorization: the browser sends the user's
password directly to Supabase Auth, then presents the resulting Supabase access
token to Aissence to approve the CLI. The CLI receives a short-lived Aissence
access token and a rotating, revocable refresh token; neither the Supabase
access token nor password is sent to the CLI.

Set `AISSENCE_NER_MODEL` to a compatible ONNX NER model. Without it, local and
server redaction still use `redact-core` pattern detection plus Aissence's
dedicated API-key, credential-assignment, and private-key detectors.

## Server

The development defaults target the local Supabase instance:

```text
DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:55422/postgres
SUPABASE_AUTH_URL=http://127.0.0.1:55421/auth/v1
AISSENCE_BIND=127.0.0.1:8787
```

The server also reads:

- `OPENROUTER_API_KEY` and optional `OPENROUTER_MODEL` / `OPENROUTER_PROVIDER`.
- `SUPABASE_JWT_SECRET` for validating browser login approval.
- `SUPABASE_ANON_KEY` for the browser consent page.
- `AISSENCE_TOKEN_SECRET`, issuer, audience, public URL, and token lifetimes.
- Configurable upload, entry, concurrency, and daily request safety limits.

Run it with:

```sh
cargo run -p aissence-server
```

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

Set `OPENROUTER_API_KEY` first. Pass `--meta-summaries` to include the extended
aggregate and overview fields.
