//! # aissence
//!
//! Correlate the chat history of AI coding agents (Claude Code, Codex,
//! OpenCode, Kimi CLI, Gemini)
//! with the git commits of a project, so you can see — for each commit — the
//! conversation that led up to it.
//!
//! Given a folder, [`analyze`]:
//!
//! 1. reads the folder's git commit log ([`git::commit_log`]),
//! 2. discovers and parses every AI session log belonging to that folder
//!    ([`sessions::load_sessions`], backed by `normalize-chat-sessions`),
//! 3. assigns each chat message to the first commit at or after it, producing a
//!    chronological list of [`CommitGroup`]s ([`coalesce::coalesce`]).
//!
//! This crate is intentionally UI-agnostic; it is the foundation for a separate
//! egui front-end.
//!
//! ```no_run
//! let history = aissence::analyze("/path/to/project")?;
//! for group in &history.groups {
//!     match &group.commit {
//!         Some(commit) => println!("{} {}", commit.short_id, commit.summary),
//!         None => println!("(uncommitted work)"),
//!     }
//!     println!("  {} messages led here", group.messages.len());
//! }
//! # Ok::<(), aissence::Error>(())
//! ```

pub mod coalesce;
pub mod error;
pub mod git;
pub mod highlight;
pub mod model;
pub mod persist;
pub mod sessions;
pub mod summarize;
pub mod ui;

use std::path::Path;

pub use error::Error;
pub use model::{
    AgentUsage, CoalescedHistory, CodeChange, CommitGroup, CommitInfo, CommitSummary, MessageKind,
    MessageSummary, TimedMessage, TokenCount, agents_for, friendly_agent,
};
pub use persist::SummaryStore;
pub use summarize::Summarizer;
pub use ui::{CodeView, DiffView};

/// Re-export of the underlying session-parsing crate, so consumers (such as the
/// egui front-end) can match on [`ncs::Role`] / [`ncs::ContentBlock`] without
/// adding their own dependency.
pub use normalize_chat_sessions as ncs;

/// Analyze a project folder: read its commits, load its AI session logs, and
/// correlate the two into a [`CoalescedHistory`].
///
/// `folder` may be the repository root or any path inside it; the enclosing git
/// repository is discovered automatically.
pub fn analyze(folder: impl AsRef<Path>) -> Result<CoalescedHistory, Error> {
    let folder = folder.as_ref();
    if !folder.exists() {
        return Err(Error::NotFound(folder.to_path_buf()));
    }

    let commits = git::commit_log(folder)?;
    let loaded = sessions::load_sessions(folder);
    Ok(coalesce::coalesce(
        folder.to_path_buf(),
        commits,
        &loaded.sessions,
        loaded.warnings,
    ))
}
