//! Public data types produced by the analysis.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use normalize_chat_sessions as ncs;
use serde::{Deserialize, Serialize};

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
    /// Model id reported by the session (e.g. `claude-opus-4-8`), if known.
    pub model: Option<String>,
    /// Friendly coding-agent name (e.g. `Claude Code`, `Codex`, `Gemini`),
    /// derived from the session format. See [`friendly_agent`].
    pub agent: Option<String>,
    /// Path of the session log file this message was read from.
    pub session_path: PathBuf,
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
    /// Friendly agent name: `Claude Code`, `Codex`, `Gemini`.
    pub provider: String,
    /// Model id reported by the session (e.g. `claude-opus-4-8`), if known.
    pub model: Option<String>,
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
        "gemini" => "Gemini",
        "normalize" => "Normalize",
        other => other,
    }
    .to_string()
}

/// The distinct (agent, model) pairs used across a commit group's messages, in
/// first-seen order.
pub fn agents_for(group: &CommitGroup) -> Vec<AgentUsage> {
    let mut usages: Vec<AgentUsage> = Vec::new();
    for message in &group.messages {
        let usage = AgentUsage {
            provider: message
                .agent
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            model: message.model.clone(),
        };
        if !usages.contains(&usage) {
            usages.push(usage);
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

/// LLM summary of one message within a commit group, plus — for `AiChanged`
/// messages — the concrete code changes it made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    pub kind: MessageKind,
    /// 2-3 sentence summary of this message (or the message verbatim if short).
    pub summary: String,
    /// Non-empty only for [`MessageKind::AiChanged`].
    pub changes: Vec<CodeChange>,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// LLM-generated summary of what went into a commit.
///
/// Produced by the [`crate::summarize::Summarizer`]. The three top-level fields
/// are the *aggregated* overview; [`messages`](Self::messages) holds the
/// per-message breakdown that drives the middle pane of the app. A field is
/// empty when the commit group has nothing of that kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommitSummary {
    /// 3-5 sentences: what the user asked for across all their messages.
    pub user_requested: String,
    /// 3-5 sentences: what the assistant explained/answered in its replies.
    pub ai_answered: String,
    /// 3-5 sentences: what the assistant implemented in code (file edits).
    pub ai_implemented: String,
    /// Per-message summaries, in chronological order.
    pub messages: Vec<MessageSummary>,
    /// Coding agent(s) + model(s) that produced this commit's sessions.
    #[serde(default)]
    pub agents: Vec<AgentUsage>,
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
}
