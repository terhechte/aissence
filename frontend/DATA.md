# The `.aissence` file format

This document describes the on-disk data in a `.aissence` file so a consumer
(for example, a viewer or web UI) can read and present it. It describes **only
the data** — its shape, types, and meaning — not how it should be displayed.

## What it is

`aissence` correlates a software project's AI coding-assistant chat sessions
(from agents such as Claude Code, Codex, OpenCode, Kimi CLI, Pi, Amp, Crush,
Antigravity, VS Code Copilot, and Gemini) with the project's git commits, and
uses an LLM to summarize — for each commit — the conversation that led to it. A
`.aissence` file is the cached, persisted record of those summaries and their
retained source text for one project.

- **Location & name:** a single file named `.aissence` at the root of the
  project folder it describes.
- **Encoding:** UTF-8 JSON, pretty-printed.
- **Unit of meaning:** one entry per git commit (keyed by commit hash). An entry
  describes the AI-assisted work — the chat conversation — that preceded that
  commit.

A commit's conversation is the set of timestamped chat messages that occurred
after the previous commit and up to that commit. Messages without a usable
timestamp cannot be assigned to a commit and are not persisted here.

The file stores both summaries and source material. For user and assistant-text
entries, the complete plain-text content is retained regardless of length.
Structured thinking blocks, tool results, system messages, and the source
session's raw JSON are not retained as transcript data. Recognized editing tool
calls are represented separately as `Change` objects.

## Top-level structure

```jsonc
{
  "version": 1,            // integer — schema version (currently 1)
  "commits": { ... },      // object — map of commit hash -> entry (see below)
  "ignored": [ ... ]       // array of strings — commit hashes deliberately skipped
}
```

| Field      | JSON type | Meaning |
|------------|-----------|---------|
| `version`  | integer   | Schema version. Currently `1`. Check it; future versions may differ. |
| `commits`  | object    | Map whose **keys are full git commit hashes** (40-char hex SHA-1 strings) and whose **values are entry objects**. Only commits that have been summarized appear here. |
| `ignored`  | array of strings | Commit hashes the user explicitly marked to skip. May be absent in older files (treat absent as empty). A hash here may or may not also appear in `commits`. |

### Commit entry (a value in `commits`)

```jsonc
{
  "signature": 17712209936...,     // integer — opaque producer cache key
  "title": "Add retry support",    // string — first line of the commit message
  "date": "2026-06-20T09:18:03Z", // string — commit time in UTC
  "stats": { ... },                // object — authoritative git diff statistics
  "summary": { ... }               // object — the summary (see "CommitSummary")
}
```

| Field       | JSON type | Meaning |
|-------------|-----------|---------|
| `signature` | integer (unsigned 64-bit) | An opaque producer cache key derived from the message timeline. It is used to decide whether a cached entry can be reused. It is not a content hash or stable external identifier, and a consumer should ignore it. It may exceed JavaScript's safe-integer range (`2^53−1`); use a JSON parser that preserves unsigned 64-bit integers if exact round-tripping is required. |
| `title`     | string | The first line of the git commit message. May be absent in older version-1 files. |
| `date`      | string | The commit's committer time in UTC, encoded as RFC 3339 / ISO 8601 (for example, `"2026-06-20T09:18:03Z"`). This is the timestamp used to correlate the conversation with the commit. May be absent in older version-1 files. |
| `stats`     | object | Authoritative file and text-line counts calculated from the committed git trees. See below. May be absent in older version-1 files. |
| `summary`   | object    | The commit's summary. See below. |

### Commit stats

```jsonc
{
  "files_changed": 3,  // integer
  "lines_added": 21,   // integer
  "lines_deleted": 4   // integer
}
```

| Field           | JSON type | Meaning |
|-----------------|-----------|---------|
| `files_changed` | integer (unsigned) | Number of files changed by the commit, including binary files and submodules. |
| `lines_added`   | integer (unsigned) | Number of text lines added by the commit. Binary files and submodules contribute no text-line count. |
| `lines_deleted` | integer (unsigned) | Number of text lines deleted by the commit. Binary files and submodules contribute no text-line count. |

The producer calculates these values from git's committed trees, comparing a
commit with its first parent (or with the empty tree for a root commit). They
describe the final commit itself and are authoritative; they are unrelated to
the best-effort, tool-call-derived `Change.diff` strings.

## `CommitSummary`

```jsonc
{
  "user_requested": "…",      // string
  "ai_answered": "…",         // string
  "ai_implemented": "…",      // string
  "messages": [ … ],          // array of message objects
  "agents": [ … ],            // array of agent objects (optional)
  "meta": "…",                // string (optional)
  "tokens": { … }             // object — token usage (optional; see "TokenCount")
}
```

| Field            | JSON type | Meaning |
|------------------|-----------|---------|
| `user_requested` | string | A concise aggregate summary of what the user asked for across the conversation that led to this commit. Usually 3–5 sentences, but it may be shorter when there is only one short source entry. Plain prose. |
| `ai_answered`    | string | A concise aggregate summary of what the assistant explained, decided, or concluded in its replies. Usually 3–5 sentences, with the same single-entry exception. |
| `ai_implemented` | string | A concise aggregate summary of what was implemented through recognized file-editing tool calls. Usually 3–5 sentences, with the same single-entry exception. |
| `meta`           | string | A single, longer cohesive overview (up to ~20 sentences) synthesizing the three fields above: what the user wanted, what was implemented, and anything important the assistant noted (limitations, caveats, trade-offs, follow-ups). **May be an empty string** (and may be absent in older files). When present it's the most complete single-text description of the commit. |
| `messages`       | array  | A chronological breakdown of conversation entries. Entries can be split or coalesced relative to source messages; see "Message". May be empty. |
| `agents`         | array  | The coding agent(s) and model(s) that produced this commit's sessions ("provenance"). See "Agent". May be empty or absent. |
| `tokens`         | object | The **exact total** token usage of the whole conversation that led to this commit — the sum of every turn's usage. See "TokenCount". **Absent** when no session reported usage for this commit (treat absent as unknown). |

Any of the three text fields may be empty if the conversation had nothing of
that kind (e.g. a commit whose conversation contained no code edits has an empty
`ai_implemented`).

## Message (an element of `messages`)

```jsonc
{
  "kind": "UserWrote",        // string enum — see below
  "summary": "…",             // string — required for UserWrote only
  "content": "…",             // string — complete source text for UserWrote/AiWrote
  "changes": [ … ],           // array of change objects
  "timestamp": "2026-06-20T11:19:46Z",  // string (RFC 3339) or null
  "tokens": { … }             // object — token usage (optional; see "TokenCount")
}
```

| Field       | JSON type | Meaning |
|-------------|-----------|---------|
| `kind`      | string    | One of `"UserWrote"`, `"AiWrote"`, `"AiChanged"` (see below). |
| `summary`   | string    | A concise summary of this user prompt. Required on every `UserWrote` entry, regardless of how many prompts belong to the commit. Absent on `AiWrote` and `AiChanged` entries. |
| `content`   | string    | The complete plain-text source underlying a `UserWrote` or `AiWrote` entry, regardless of length. If consecutive source entries were coalesced, their texts are joined with a blank line. Required for those two kinds and absent for `AiChanged`. Consumers can derive a short display preview directly from this text without an LLM-generated per-message summary. |
| `changes`   | array     | The concrete code edits this message made. **Non-empty only when `kind` is `"AiChanged"`**; an empty array otherwise. See "Change". |
| `timestamp` | string \| null | When the entry occurred, as an RFC 3339 / ISO 8601 UTC timestamp (e.g. `"2026-06-20T11:19:46Z"`, sometimes with fractional seconds like `"…46.071Z"`). A coalesced entry keeps the first source timestamp. `null` is permitted for compatibility, although newly generated commit entries come from timestamped messages. |
| `tokens`    | object | Token usage **attributed** to this entry. See "TokenCount". It is turn-level rather than a fine-grained measurement of this individual entry. Absent when no usage was attributed. Consumers should treat the commit-level total as authoritative rather than relying on message values to reconstruct it. |

### `kind` values

| Value         | Meaning |
|---------------|---------|
| `"UserWrote"` | One or more consecutive plain-text messages from the human user (instructions, questions, or reports). |
| `"AiWrote"`   | One or more consecutive plain-text replies from the assistant (explanations, decisions, or conclusions). |
| `"AiChanged"` | One or more consecutive recognized file-editing actions from the assistant. Its `changes` array holds the edits. |

One source assistant message can produce both an `AiWrote` entry for its text and
an `AiChanged` entry for its editing tool calls. After that split, consecutive
entries of the same `kind` are merged. For example, a run of code-editing steps
appears as one `AiChanged` entry whose `changes` array contains all edits.
Entries are in chronological order.

## Change (an element of a message's `changes`)

```jsonc
{
  "tool": "Edit",                 // string
  "file": "src/foo.rs",           // string
  "diff": "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ …"   // string (unified diff)
}
```

| Field  | JSON type | Meaning |
|--------|-----------|---------|
| `tool` | string | The editing tool the assistant used, as named by its agent (e.g. `"Edit"`, `"Write"`, `"MultiEdit"`, `"write_file"`, `"replace"`, `"apply_patch"`). |
| `file` | string | The file the change targets. Usually a project-relative path; may be `"(unknown file)"` if the tool call didn't name one. |
| `diff` | string | A best-effort, display-oriented unified-diff-like rendering synthesized from the tool-call payload. For tools that provide old/new text, the `-`/`+` content is faithful but line numbers are nominal. New-file content is rendered against `/dev/null`. Other payload shapes may produce nonstandard hunk headers or may be represented as added payload text, so this string is not guaranteed to be an applicable patch or an authoritative record of the final file. |

## Agent (an element of `agents`)

```jsonc
{
  "provider": "Claude Code",      // string
  "model": "claude-opus-4-8",     // string or null
  "tokens": {                     // object (optional)
    "input": 12345,
    "output": 4096,
    "cache_read": 8000
  }
}
```

| Field      | JSON type | Meaning |
|------------|-----------|---------|
| `provider` | string | Friendly name of the coding agent that produced the session, such as `"Claude Code"`, `"Codex"`, `"OpenCode"`, `"Kimi CLI"`, `"Pi"`, `"Oh My Pi"`, `"Amp"`, `"Crush"`, `"Antigravity"`, `"VS Code Copilot"`, or `"Gemini"` (other values are possible). This is the *agent/tool*, not the underlying LLM vendor. |
| `model`    | string \| null | The raw model identifier reported by the session (e.g. `"claude-opus-4-8"`, a GPT id, a Gemini id), or `null` if unknown. This is a literal id, not a marketing name. |
| `tokens`   | object | The exact token usage attributed to this `(provider, model)` pair for the commit. See "TokenCount". Optional for compatibility with older files. |

`agents` lists the **distinct** `(provider, model)` pairs seen across the
commit's conversation, in first-seen order. A single commit can list more than
one — for example if a subagent ran on a different model. When every agent has
`tokens`, those per-agent counts sum to `summary.tokens` and let consumers price
each model's usage at its own rate.

## TokenCount (a `tokens` object)

```jsonc
{
  "input": 12345,          // integer
  "output": 4096,          // integer
  "cache_read": 8000,      // integer (optional)
  "cache_create": 1200     // integer (optional)
}
```

| Field          | JSON type | Meaning |
|----------------|-----------|---------|
| `input`        | integer (unsigned) | Input (prompt) tokens. |
| `output`       | integer (unsigned) | Output (completion) tokens. |
| `cache_read`   | integer (unsigned) | Tokens read from the prompt cache. **Absent** when the agent didn't report it. |
| `cache_create` | integer (unsigned) | Tokens written to the prompt cache. **Absent** when the agent didn't report it. |

The same shape is used in three places, with different guarantees:

- **On a `CommitSummary` (`summary.tokens`)** it is the **exact** total for the
  whole conversation that led to the commit — the sum of every turn's usage.
- **On an `Agent` (`agent.tokens`)** it is the exact subtotal attributed to that
  `(provider, model)` pair for the commit. Complete per-agent subtotals sum to
  `summary.tokens`; older files may omit them.
- **On a `Message` (`message.tokens`)** it is an attribution of turn-level usage,
  not a tokenization of that individual entry. The producer normally places a
  turn's usage on the assistant entry the turn produced, but can fall back to
  another entry when no assistant entry is available.

Counts are cumulative sums of what each agent reported; they are not re-tokenized
or re-priced. A commit (or message) with no reported usage omits `tokens`.
Normally the message attributions sum to the commit total, but consumers should
use `summary.tokens` directly: older/backfilled files and source turns that do
not produce a persisted text/change entry need not satisfy that equality.

## What is *not* in this file

A consumer should know the file's boundaries:

- **Limited git metadata.** The commit title (first message line), committer
  date, and aggregate diff stats are stored. The full commit message, author,
  and parents are not; those must be read from the git repository if needed.
- **No full commit diff.** Only the per-edit synthesized `diff`s inside
  `changes` are present. The authoritative aggregate stats of the whole commit
  are stored, but its patch is not; the patch would come from git.
- **No complete structured transcript.** Full normalized plain-text content is
  retained for user and assistant-text entries, so the file can contain
  sensitive source prompts and replies. It does not retain structured thinking
  blocks, tool results, system messages, non-editing tool calls, or the source
  session's raw JSON.
- **Only summarized commits appear.** A commit with no entry in `commits` simply
  hasn't been summarized. There is no list of "all commits" here.

## Robustness notes for consumers

- Treat missing `ignored` and `agents` as empty collections and missing `meta`
  as an empty string.
- Treat missing `content` as unavailable source text, not as an empty original
  message. Older files do not contain this field.
- Treat missing `title`, `date`, or `stats` as unavailable git metadata. Older
  version-1 files do not contain these fields.
- Treat missing `tokens` as unknown usage, not as zero usage.
- Tolerate unknown fields that may be added in future versions — ignore them.
- Inspect `version`; handle unknown versions gracefully.
- Object/field order is not significant.
- Strings can contain newlines (the summaries are multi-sentence prose; `diff`
  values are multi-line code). They are plain text, not HTML.

## Complete example

```json
{
  "version": 1,
  "commits": {
    "8f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2d": {
      "signature": 17712209936023451233,
      "title": "Add retry support to the network client",
      "date": "2026-06-20T09:18:03Z",
      "stats": {
        "files_changed": 3,
        "lines_added": 48,
        "lines_deleted": 7
      },
      "summary": {
        "user_requested": "The user asked to add a retry wrapper around the network client and make the timeout configurable.",
        "ai_answered": "The assistant proposed an exponential-backoff retry helper and explained that the timeout would be read from an environment variable with a sensible default.",
        "ai_implemented": "Added a retry() helper in net/retry.rs, wired it into the client's request path, and read REQUEST_TIMEOUT from the environment in config.rs.",
        "messages": [
          {
            "kind": "UserWrote",
            "summary": "Requested a retry wrapper for the network client and a configurable timeout.",
            "content": "Please add retries around the network client and make its timeout configurable.",
            "changes": [],
            "timestamp": "2026-06-20T09:14:02.118Z"
          },
          {
            "kind": "AiWrote",
            "content": "I’ll add a small exponential-backoff helper around the request path. The timeout can come from REQUEST_TIMEOUT with a default when it is unset.",
            "changes": [],
            "timestamp": "2026-06-20T09:14:20Z",
            "tokens": { "input": 5820, "output": 240, "cache_read": 4096 }
          },
          {
            "kind": "AiChanged",
            "changes": [
              {
                "tool": "Write",
                "file": "src/net/retry.rs",
                "diff": "--- /dev/null\n+++ b/src/net/retry.rs\n@@ -0,0 +1,3 @@\n+pub fn retry() {\n+    // backoff\n+}\n"
              },
              {
                "tool": "Edit",
                "file": "src/config.rs",
                "diff": "--- a/src/config.rs\n+++ b/src/config.rs\n@@ -1,1 +1,2 @@\n-let timeout = 30;\n+let timeout = std::env::var(\"REQUEST_TIMEOUT\").ok();\n"
              }
            ],
            "timestamp": "2026-06-20T09:16:41Z",
            "tokens": { "input": 7010, "output": 512, "cache_read": 6400 }
          }
        ],
        "agents": [
          { "provider": "Claude Code", "model": "claude-opus-4-8" }
        ],
        "meta": "The user wanted the network client to be more resilient: a retry wrapper plus a configurable request timeout. The assistant settled on exponential backoff and a timeout sourced from the REQUEST_TIMEOUT environment variable, defaulting when unset. It implemented a small retry() helper, integrated it into the request path, and replaced the hard-coded timeout with an environment lookup. The assistant noted that the backoff parameters are fixed for now and could later be made configurable.",
        "tokens": { "input": 12830, "output": 752, "cache_read": 10496 }
      }
    },
    "1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4": {
      "signature": 4490112233445566778,
      "title": "Fix README typo",
      "date": "2026-06-21T15:03:22Z",
      "stats": {
        "files_changed": 1,
        "lines_added": 1,
        "lines_deleted": 1
      },
      "summary": {
        "user_requested": "The user asked to fix a typo in the README.",
        "ai_answered": "",
        "ai_implemented": "Corrected a misspelling in README.md.",
        "messages": [
          {
            "kind": "UserWrote",
            "summary": "Reported a typo in the README.",
            "content": "There is a typo in the README: “recieve” should be “receive.”",
            "changes": [],
            "timestamp": null
          },
          {
            "kind": "AiChanged",
            "changes": [
              {
                "tool": "Edit",
                "file": "README.md",
                "diff": "--- a/README.md\n+++ b/README.md\n@@ -1,1 +1,1 @@\n-recieve\n+receive\n"
              }
            ],
            "timestamp": "2026-06-21T15:02:10Z"
          }
        ],
        "agents": [
          { "provider": "Codex", "model": "gpt-5.1-codex" }
        ],
        "meta": "A small documentation fix: the user pointed out a typo in the README, and the assistant corrected \"recieve\" to \"receive\". No code behavior changed."
      }
    }
  },
  "ignored": [
    "deadbeefcafef00d1234567890abcdef12345678"
  ]
}
```
