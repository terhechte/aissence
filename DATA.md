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
      "sanitizer_version": 1,
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

- `user_requested`
- `ai_answered`
- `ai_implemented`
- `meta`
- chronological `messages`, with generated summaries only on `UserWrote`
  entries
- `agents` and `tokens`

The summary message list must have the same length and kinds as `source.messages`.

## Redaction

Before persistence and upload, Aissence runs `redact-core`, optional
`redact-ner` when `AISSENCE_NER_MODEL` is configured, and deterministic secret
recognizers. The server runs the same scanner again and rejects an upload if it
would change under redaction. Server-generated summaries are scanned before
being written to Postgres or returned to the client.

The format is intentionally strict: unknown fields, unsupported versions,
invalid digests, oversized entries, and mismatched summary shapes are rejected.
