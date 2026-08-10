# The `.aissence` version-2 format

`.aissence` is UTF-8 JSON containing sanitized source evidence and optional
summaries for one project. An unsummarized document is complete and can be sent
to the Aissence server; a locally summarized document can also be uploaded
manually without another model call.

```jsonc
{
  "version": 2,
  "project": {
    "source_id": "e2df2c01-3c7e-4f54-b605-fce2a6159170",
    "name": "aissence",
    "repository": "github.com/owner/aissence"
  },
  "commits": [
    {
      "commit_hash": "40 lowercase hexadecimal characters",
      "source_digest": "sha256:...",
      "sanitizer_version": 2,
      "title": "Add server sync",
      "body": null,
      "date": "2026-08-01T12:00:00Z",
      "stats": {
        "files_changed": 4,
        "lines_added": 100,
        "lines_deleted": 20
      },
      "files": [],
      "source": {
        "messages": [],
        "agents": [],
        "tokens": null
      },
      "summary": null
    }
  ],
  "ignored": []
}
```

## Project

`source_id` is a random UUID generated on the first local analysis and retained
across rewrites. It is the stable per-owner server identity even when a remote
repository is renamed. Repository metadata never contains credentials or a
protocol prefix.

## Commit source and digest

Commits are chronological. `source` contains the model-independent input:

- `messages`: coalesced `UserWrote`, `AiWrote`, and `AiChanged` entries.
- `agents`: coding-agent/model provenance.
- `tokens`: source-agent token usage when reported.

User and assistant text is retained in `messages[].content`; tool-derived code
changes use `messages[].changes`. Author name/email, system messages, thinking,
tool results, raw session JSON, and full committed diffs are not persisted.

`source_digest` is SHA-256 over canonical serialized commit metadata and the
sanitized source. It excludes `summary`, so merging a server result does not
make the commit appear changed. The digest includes `sanitizer_version`, making
redaction upgrades explicit synchronization changes.

## Summary

`summary` is absent or `null` until generated. When present it contains:

- `public.summary`: the single privacy-safe commit narrative
- `public.subline`: a legacy compatibility field; current generation leaves it empty
- `public.beats`: zero to three representative, paraphrased prompt moments
- `public.origin`: `model` or the explicit metadata/parse/privacy fallback used
- `summarizer_version`: the prompt, evidence, and validation policy version
- chronological `messages`; `subscriber_summary` is optional and only valid on
  `UserWrote` entries
- `prompt_failures`: unresolved message indexes with the attempted model,
  validation/upstream error, and attempt timestamp
- `agents` and `tokens`

The summary message list must have the same length and kinds as `source.messages`.
Normal public generation first creates privacy-safe summaries for every user
prompt, then asks for an evidence-scaled story and beats using those summaries.
The intermediate results remain ephemeral and every `subscriber_summary` stays
absent. Subscriber summaries are normally a separate on-demand tier;
`--all-prompts` stores and publishes the public-safe prompt summaries already
created by this pipeline and asks only for the story without unused beats.

## Hosted generation lifecycle

Uploading stores sanitized source and completes the import immediately. Draft
projects do not create public summary jobs automatically. A creator explicitly
generates and reviews private summaries before publishing; publication is
blocked while prompted commits are pending, failed, incomplete, or outdated.
For raw imports, the public job summarizes every non-empty user prompt in
batches of at most eight entries/6,000 characters. It then synthesizes the
story from those privacy-safe summaries, the bounded final assistant reply,
and a metadata-only git fingerprint. The synthesis sections reserve 16,000,
7,000, and 7,000 characters respectively inside a 32,000-character ceiling.
If prompt summaries exceed their section, complete first/final entries and
correction- or information-rich intermediates are prioritized. Raw user text
and diff bodies are not sent to the synthesis call.

Local generation with `--all-prompts` additionally fills every user message's
`subscriber_summary`. When a complete set is uploaded, those summaries are
marked public and rendered for public visitors; raw prompt sources remain
restricted. Hosted subscriber summaries do not receive this public marker.

The first active or trialing subscriber for a creator queues subscriber jobs for
the creator's published commits. Those jobs summarize every user prompt in
batches of at most eight/6,000 characters; prompts of 200 characters or fewer
are copied without a model request because the entitled reader may already read
the complete source. Batch responses carry the original message indexes and
use a strict JSON schema on OpenRouter. Shape, count, index, empty-output, and
privacy-overlap failures are reported separately. A failed multi-prompt batch
falls back to individual requests; successful siblings are stored even when
another prompt remains unresolved. Subscriber work never rewrites public copy.
Later imports queue both tiers only when the creator still has an active
subscriber.

Prompt-summary length is normalized independently from the public commit story.
Prompts of at most five words target 4–12 words, prompts of six to twelve words
target 6–18, prompts up to 60 words target 12–35, and longer prompts target
35–60. Validation accepts wider 3–24, 4–24, 8–45, and 20–75-word safety bands
respectively. A configured fallback model handles request, shape, index,
emptiness, extreme word-count, or privacy failures before work is recorded as
unresolved.

Public-story targets grow with the bounded synthesis evidence: 60–90 words at
the low end through 120–150 words at the high end. Validation uses a stable
50–165-word safety envelope, avoiding a metadata fallback merely because
otherwise-good copy missed its preferred evidence-scaled target.

## Redaction

Before persistence and upload, Aissence runs `redact-core`, optional
`redact-ner` when `AISSENCE_NER_MODEL` is configured, and deterministic secret
recognizers. The server runs the same scanner again and rejects an upload if it
would change under redaction. Server-generated summaries are scanned before
being written to Postgres or returned to the client.

Authoritative commit statistics are protected while generated summary text is
redacted, so file counts and added/deleted line counts bypass person/location
NER. For example, `23524` remains a line count instead of becoming a location
placeholder.

The format is intentionally strict: unknown fields, unsupported versions,
invalid digests, oversized entries, and mismatched summary shapes are rejected.
