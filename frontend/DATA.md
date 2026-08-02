# `.aissence` v2 data contract

The frontend accepts only the strict version-2 document also consumed by the
Rust server. `project.source_id` is the stable UUID for a local project.
`commits` is a chronological array; every commit contains a verified
`source_digest`, sanitizer version, git metadata, and a `source` object.

`source.messages` contains coalesced `UserWrote`, `AiWrote`, and `AiChanged`
entries. Text entries retain sanitized `content`; change entries retain tool,
file, and diff records. `source.agents` and `source.tokens` carry coding-agent
provenance and observed usage. Author identity, system messages, thinking, tool
results, and raw logs are not part of the format.

`summary` is optional. When present it has this shape:

```json
{
  "public": {
    "summary": "A privacy-safe account of why and how this commit happened.",
    "beats": [
      {
        "message_index": 0,
        "label": "Initial request",
        "text": "A paraphrased prompt moment without private wording."
      }
    ]
  },
  "messages": [
    {
      "kind": "UserWrote",
      "subscriber_summary": "Optional paid-tier prompt summary.",
      "content": "Sanitized full source",
      "changes": [],
      "timestamp": null
    }
  ],
  "agents": [],
  "tokens": { "input": 0, "output": 0 }
}
```

The summary message list must match `source.messages` by length and kind. Public
beats may reference only user-message indexes and are capped at three.
`subscriber_summary` is valid only on user messages and normally remains absent
until the paid-tier job runs.

Postgres stores the public narrative on `commits`, private summaries in
`message_summaries`, and complete retained text in `message_sources`. This
physical split supports explicit audience-safe queries and RLS policies.
