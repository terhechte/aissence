//! Public data types produced by the analysis.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use normalize_chat_sessions as ncs;
use serde::{Deserialize, Serialize};

/// Authoritative statistics for the tree change introduced by a git commit.
///
/// Counts are calculated from the committed trees (against the first parent,
/// or the empty tree for a root commit), not from assistant tool calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStats {
    /// Number of changed files, including binary files and submodules.
    pub files_changed: u64,
    /// Number of text lines added.
    pub lines_added: u64,
    /// Number of text lines deleted.
    pub lines_deleted: u64,
}

/// A single git commit, normalized for correlation with chat messages.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Full hex object id.
    pub id: String,
    /// Abbreviated id (honoring `core.abbrev`).
    pub short_id: String,
    /// First line of the commit message.
    pub summary: String,
    /// Remainder of the commit message, if any.
    pub body: Option<String>,
    /// Full, raw commit message (summary + body).
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    /// Committer time, in UTC. This is the timestamp used for correlation.
    pub time: DateTime<Utc>,
    /// Authoritative file/line statistics from the committed tree diff.
    pub stats: CommitStats,
}

/// One chat message lifted out of a parsed session, carrying just enough
/// session context to render and correlate it.
///
/// The original content blocks are preserved verbatim so a UI layer can render
/// tool calls, tool results and thinking blocks — not only plain text.
#[derive(Debug, Clone)]
pub struct TimedMessage {
    /// Parsed message timestamp in UTC, if the source provided a usable one.
    pub timestamp: Option<DateTime<Utc>>,
    pub role: ncs::Role,
    pub content: Vec<ncs::ContentBlock>,
    /// Session identifier this message belongs to (format-specific).
    pub session_id: Option<String>,
    /// LLM provider that produced the session (e.g. `anthropic`, `openai`).
    pub provider: Option<String>,
    /// Model id reported for this message's turn (e.g. `claude-opus-4-8`),
    /// falling back to the session-wide model when turn-level data is absent.
    pub model: Option<String>,
    /// Friendly coding-agent name (e.g. `Claude Code`, `Codex`, `OpenCode`),
    /// derived from the session format. See [`friendly_agent`].
    pub agent: Option<String>,
    /// Path of the session log file this message was read from.
    pub session_path: PathBuf,
    /// Token usage attributed to this message. The agents report usage per
    /// *turn*, so this is populated only on the assistant message a turn
    /// produced (the reply that generated the output tokens); the other
    /// messages of the turn carry `None`. Summing it across a group yields the
    /// group's exact total. `None` when the source reported no usage.
    pub token_usage: Option<TokenCount>,
}

impl TimedMessage {
    /// Concatenate all plain-text blocks of this message.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let ncs::ContentBlock::Text { text } = block {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }

    /// Concatenate all extended-thinking blocks of this message.
    pub fn thinking(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let ncs::ContentBlock::Thinking { text } = block {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }

    /// Names of the tools this message invokes, in order.
    pub fn tool_uses(&self) -> Vec<&str> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ncs::ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Input/output token counts for a message, agent/model pair, or whole
/// commit's conversation.
///
/// The agents report usage per *turn* (one request/response cycle), not per
/// individual message. Per-commit totals are therefore exact sums of those
/// turns; per-message counts are the turn's usage attributed to the assistant
/// message the turn produced (approximate — see `DATA.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCount {
    /// Input (prompt) tokens.
    pub input: u64,
    /// Output (completion) tokens.
    pub output: u64,
    /// Tokens read from the prompt cache, if the agent reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// Tokens written to the prompt cache, if the agent reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_create: Option<u64>,
}

impl TokenCount {
    /// `true` when every count is zero (no usage recorded).
    pub fn is_zero(&self) -> bool {
        self.input == 0
            && self.output == 0
            && self.cache_read.unwrap_or(0) == 0
            && self.cache_create.unwrap_or(0) == 0
    }

    /// Accumulate another count into this one (summing the optional cache
    /// fields, which stay `None` only while neither side reported them).
    pub fn add(&mut self, other: &TokenCount) {
        self.input += other.input;
        self.output += other.output;
        if let Some(v) = other.cache_read {
            *self.cache_read.get_or_insert(0) += v;
        }
        if let Some(v) = other.cache_create {
            *self.cache_create.get_or_insert(0) += v;
        }
    }
}

impl From<&ncs::TokenUsage> for TokenCount {
    fn from(usage: &ncs::TokenUsage) -> Self {
        TokenCount {
            input: usage.input,
            output: usage.output,
            cache_read: usage.cache_read,
            cache_create: usage.cache_create,
        }
    }
}

/// What a single summarized message represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageKind {
    /// A user message ("user wrote …").
    UserWrote,
    /// An assistant text reply ("ai wrote …").
    AiWrote,
    /// An assistant message that edited code ("ai changed …").
    AiChanged,
}

impl MessageKind {
    pub fn label(self) -> &'static str {
        match self {
            MessageKind::UserWrote => "User wrote",
            MessageKind::AiWrote => "AI wrote",
            MessageKind::AiChanged => "AI changed",
        }
    }
}

/// The coding agent + model used in a session — a commit's "provenance".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentUsage {
    /// Friendly agent name: `Claude Code`, `Codex`, `OpenCode`, `Gemini`.
    pub provider: String,
    /// Model id reported by the session (e.g. `claude-opus-4-8`), if known.
    pub model: Option<String>,
    /// Exact token usage from turns attributed to this agent/model pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenCount>,
}

impl AgentUsage {
    /// `"Claude Code · claude-opus-4-8"` (or just the provider if no model).
    pub fn label(&self) -> String {
        match &self.model {
            Some(model) => format!("{} · {}", self.provider, model),
            None => self.provider.clone(),
        }
    }
}

/// Map a `normalize-chat-sessions` format id to a friendly agent name.
pub fn friendly_agent(format: &str) -> String {
    match format {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "kimi" => "Kimi CLI",
        "pi" => "Pi",
        "omp" => "Oh My Pi",
        "amp" => "Amp",
        "crush" => "Crush",
        "antigravity" => "Antigravity",
        "vscode-copilot" => "VS Code Copilot",
        "gemini" => "Gemini",
        "normalize" => "Normalize",
        other => other,
    }
    .to_string()
}

/// The distinct (agent, model) pairs used across a commit group's messages, in
/// first-seen order, with exact per-pair token totals where reported.
pub fn agents_for(group: &CommitGroup) -> Vec<AgentUsage> {
    let mut usages: Vec<AgentUsage> = Vec::new();
    for message in &group.messages {
        let provider = message
            .agent
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        if let Some(usage) = usages
            .iter_mut()
            .find(|usage| usage.provider == provider && usage.model == message.model)
        {
            if let Some(tokens) = message.token_usage {
                usage
                    .tokens
                    .get_or_insert_with(TokenCount::default)
                    .add(&tokens);
            }
        } else {
            usages.push(AgentUsage {
                provider,
                model: message.model.clone(),
                tokens: message.token_usage,
            });
        }
    }
    usages
}

/// A single code edit the assistant made, rendered as a unified diff so it can
/// be shown in [`crate::DiffView`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChange {
    /// Editing tool that produced the change (e.g. `Edit`, `Write`).
    pub tool: String,
    /// File the change applies to.
    pub file: String,
    /// Synthesized unified-diff text (feed to [`crate::DiffView::from_patch`]).
    pub diff: String,
}

/// One conversation entry within a commit group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    pub kind: MessageKind,
    /// LLM-generated summary for [`MessageKind::UserWrote`]. Newly generated
    /// assistant-text and editing entries always leave this absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Complete source text for user and assistant-text entries, regardless of
    /// length. Consecutive source messages represented by one coalesced entry
    /// are joined with blank lines. Absent for code-change entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Non-empty only for [`MessageKind::AiChanged`].
    pub changes: Vec<CodeChange>,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    /// Token usage attributed to this message (turn-level, approximate — see
    /// [`TokenCount`]). Absent when the source reported no usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenCount>,
}

/// LLM-generated summary of what went into a commit.
///
/// Produced by the [`crate::summarize::Summarizer`]. The three top-level fields
/// are the *aggregated* overview; [`messages`](Self::messages) holds a
/// breakdown that drives the middle pane of the app. Only user entries receive
/// per-entry LLM summaries. A field is empty when the commit group has nothing
/// of that kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommitSummary {
    /// 3-5 sentences: what the user asked for across all their messages.
    pub user_requested: String,
    /// 3-5 sentences: what the assistant explained/answered in its replies.
    pub ai_answered: String,
    /// 3-5 sentences: what the assistant implemented in code (file edits).
    pub ai_implemented: String,
    /// Locally-derived conversation entries, in chronological order.
    pub messages: Vec<MessageSummary>,
    /// Coding agent(s) + model(s) that produced this commit's sessions,
    /// including exact per-pair token totals when reported.
    #[serde(default)]
    pub agents: Vec<AgentUsage>,
    /// A single cohesive overview (up to ~20 sentences) synthesizing what the
    /// user wanted, what the AI implemented, and any important caveats. Built
    /// last, from the three fields above.
    #[serde(default)]
    pub meta: String,
    /// Exact total token usage of the whole conversation that led to this
    /// commit (the sum of every turn's usage). Absent when no session reported
    /// usage for this commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenCount>,
}

/// A commit together with the chat messages that led up to it.
///
/// `commit` is `None` for the trailing group: messages that occurred after the
/// most recent commit and have therefore not (yet) resulted in a commit.
#[derive(Debug, Clone)]
pub struct CommitGroup {
    pub commit: Option<CommitInfo>,
    /// Messages assigned to this commit, in chronological order.
    pub messages: Vec<TimedMessage>,
    /// LLM summary of this group, populated on demand by
    /// [`crate::summarize::Summarizer`]. `None` until summarized.
    pub summary: Option<CommitSummary>,
}

impl CommitGroup {
    /// `true` for the trailing "work in progress" group.
    pub fn is_pending(&self) -> bool {
        self.commit.is_none()
    }

    /// Number of user prompts in this group.
    ///
    /// Assistant replies, tool activity, and system records remain available
    /// in [`messages`](Self::messages), but do not inflate this user-facing
    /// conversation count.
    pub fn prompt_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| message.role == ncs::Role::User)
            .count()
    }

    /// Exact total token usage across this group's messages — the conversation
    /// that led to the commit. `None` when no message recorded any usage. This
    /// sums the per-turn counts the agents reported and is independent of
    /// whether the group has been summarized.
    pub fn token_usage(&self) -> Option<TokenCount> {
        let mut total = TokenCount::default();
        let mut any = false;
        for message in &self.messages {
            if let Some(tokens) = &message.token_usage {
                total.add(tokens);
                any = true;
            }
        }
        any.then_some(total)
    }
}

/// The full result of correlating a folder's commits and chat sessions.
#[derive(Debug, Clone)]
pub struct CoalescedHistory {
    /// The folder that was analyzed.
    pub folder: PathBuf,
    /// Commit groups in chronological (oldest-first) order. A trailing group
    /// with `commit == None` holds post-last-commit messages, if any.
    pub groups: Vec<CommitGroup>,
    /// Messages that could not be placed on the timeline because they lacked a
    /// parseable timestamp.
    pub undated: Vec<TimedMessage>,
    /// Non-fatal problems encountered while loading sessions (e.g. a file that
    /// failed to parse). Useful to surface in a UI without aborting analysis.
    pub warnings: Vec<String>,
}

impl CoalescedHistory {
    /// Total number of chat messages placed into commit groups.
    pub fn message_count(&self) -> usize {
        self.groups.iter().map(|g| g.messages.len()).sum()
    }

    /// Total number of user prompts placed into commit groups.
    pub fn prompt_count(&self) -> usize {
        self.groups.iter().map(CommitGroup::prompt_count).sum()
    }
}
