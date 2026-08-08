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
- `public.subline`: the compact summary shown beneath the commit title
- `public.beats`: zero to three representative, paraphrased prompt moments
- `public.origin`: `model` or the explicit metadata/parse/privacy fallback used
- `summarizer_version`: the prompt, evidence, and validation policy version
- chronological `messages`; `subscriber_summary` is optional and only valid on
  `UserWrote` entries
- `agents` and `tokens`

The summary message list must have the same length and kinds as `source.messages`.
Normal public generation makes one request per commit for the story, subline,
and beats together, and leaves every `subscriber_summary` absent. Subscriber summaries are normally a separate
on-demand tier; `--all-prompts` is the explicit exception that publishes a
complete locally generated set.

## Hosted generation lifecycle

Uploading stores sanitized source and completes the import immediately. Draft
projects do not create public summary jobs automatically. A creator explicitly
generates and reviews private summaries before publishing; publication is
blocked while prompted commits are pending, failed, incomplete, or outdated.
The public job receives separately bounded evidence: first/final user prompts,
up to two correction-like intermediate prompts, the bounded final assistant
reply, and a metadata-only git fingerprint. The sections reserve 16,000,
7,000, and 7,000 characters respectively inside a 32,000-character ceiling.
Oversized entries retain both ends. Diff bodies are not sent.

Local generation with `--all-prompts` additionally fills every user message's
`subscriber_summary`. When a complete set is uploaded, those summaries are
marked public and rendered for public visitors; raw prompt sources remain
restricted. Hosted subscriber summaries do not receive this public marker.

The first active or trialing subscriber for a creator queues subscriber jobs for
the creator's published commits. Those jobs summarize every user prompt in
batches of at most eight/6,000 characters; prompts of 200 characters or fewer
are copied without a model request because the entitled reader may already read
the complete source. Batch responses carry the original message indexes and
are rejected if their order/shape cannot be matched. Subscriber work never
rewrites public copy. Later imports queue both tiers only when the creator still
has an active subscriber.

## Redaction

Before persistence and upload, Aissence runs `redact-core`, optional
`redact-ner` when `AISSENCE_NER_MODEL` is configured, and deterministic secret
recognizers. The server runs the same scanner again and rejects an upload if it
would change under redaction. Server-generated summaries are scanned before
being written to Postgres or returned to the client.

The format is intentionally strict: unknown fields, unsupported versions,
invalid digests, oversized entries, and mismatched summary shapes are rejected.
