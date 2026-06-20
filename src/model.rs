//! Public data types produced by the analysis.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use normalize_chat_sessions as ncs;

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
    /// Provider that produced the session (e.g. `anthropic`, `openai`, `google`).
    pub provider: Option<String>,
    pub model: Option<String>,
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

/// A commit together with the chat messages that led up to it.
///
/// `commit` is `None` for the trailing group: messages that occurred after the
/// most recent commit and have therefore not (yet) resulted in a commit.
#[derive(Debug, Clone)]
pub struct CommitGroup {
    pub commit: Option<CommitInfo>,
    /// Messages assigned to this commit, in chronological order.
    pub messages: Vec<TimedMessage>,
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
