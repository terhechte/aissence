//! Correlating chat messages with the commits they led to.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use normalize_chat_sessions as ncs;

use crate::model::{CoalescedHistory, CommitGroup, CommitInfo, TimedMessage};

/// Coalesce commits and parsed sessions into a chronological history.
///
/// Each timestamped message is assigned to the **first commit at or after** its
/// timestamp — i.e. the commit that the conversation up to that point led to.
/// Messages occurring after the most recent commit form a trailing group with
/// `commit == None` (work that hasn't produced a commit yet).
///
/// Messages without a parseable timestamp can't be placed on the timeline and
/// are returned separately in [`CoalescedHistory::undated`].
pub fn coalesce(
    folder: PathBuf,
    mut commits: Vec<CommitInfo>,
    sessions: &[ncs::Session],
    warnings: Vec<String>,
) -> CoalescedHistory {
    let (mut timed, undated) = flatten_messages(sessions);

    timed.sort_by_key(|m| m.timestamp);
    commits.sort_by_key(|c| c.time);

    let mut groups = Vec::with_capacity(commits.len() + 1);
    let mut idx = 0;

    // Two-pointer sweep: both lists are sorted ascending by time.
    for commit in commits {
        let mut messages = Vec::new();
        while idx < timed.len() {
            // `timestamp` is always `Some` here — undated messages were split off.
            let when = timed[idx].timestamp.expect("timed message has timestamp");
            if when <= commit.time {
                messages.push(timed[idx].clone());
                idx += 1;
            } else {
                break;
            }
        }
        groups.push(CommitGroup {
            commit: Some(commit),
            messages,
            summary: None,
        });
    }

    // Anything left occurred after the last commit.
    if idx < timed.len() {
        groups.push(CommitGroup {
            commit: None,
            messages: timed[idx..].to_vec(),
            summary: None,
        });
    }

    CoalescedHistory {
        folder,
        groups,
        undated,
        warnings,
    }
}

/// Flatten every message of every session into [`TimedMessage`]s, splitting them
/// by whether a usable timestamp could be parsed.
fn flatten_messages(sessions: &[ncs::Session]) -> (Vec<TimedMessage>, Vec<TimedMessage>) {
    let mut timed = Vec::new();
    let mut undated = Vec::new();

    for session in sessions {
        for turn in &session.turns {
            for message in &turn.messages {
                let timestamp = message.timestamp.as_deref().and_then(parse_timestamp);
                let tm = TimedMessage {
                    timestamp,
                    role: message.role,
                    content: message.content.clone(),
                    session_id: session.metadata.session_id.clone(),
                    provider: session.metadata.provider.clone(),
                    model: session.metadata.model.clone(),
                    session_path: session.path.clone(),
                };
                if timestamp.is_some() {
                    timed.push(tm);
                } else {
                    undated.push(tm);
                }
            }
        }
    }

    (timed, undated)
}

/// Best-effort parse of the heterogeneous timestamp strings the various agents
/// emit. Handles RFC 3339 (the common case) and raw unix epoch seconds/millis.
fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }

    if let Ok(n) = raw.parse::<i64>() {
        // Disambiguate seconds vs milliseconds by magnitude.
        if n >= 1_000_000_000_000 {
            return DateTime::from_timestamp_millis(n);
        }
        if n >= 1_000_000_000 {
            return DateTime::from_timestamp(n, 0);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn commit(summary: &str, hour: u32) -> CommitInfo {
        CommitInfo {
            id: format!("{summary}-id"),
            short_id: summary.into(),
            summary: summary.into(),
            body: None,
            message: summary.into(),
            author_name: "tester".into(),
            author_email: "t@example.com".into(),
            time: Utc.with_ymd_and_hms(2026, 1, 1, hour, 0, 0).unwrap(),
        }
    }

    fn user_msg(text: &str, timestamp: Option<&str>) -> ncs::Message {
        ncs::Message {
            role: ncs::Role::User,
            content: vec![ncs::ContentBlock::Text { text: text.into() }],
            timestamp: timestamp.map(Into::into),
        }
    }

    /// Wrap messages into a single-session slice for [`coalesce`].
    fn session_with(messages: Vec<ncs::Message>) -> ncs::Session {
        let mut session = ncs::Session::new(PathBuf::from("test.jsonl"), "test");
        session.turns.push(ncs::Turn {
            messages,
            token_usage: None,
        });
        session
    }

    fn texts(group: &CommitGroup) -> Vec<String> {
        group.messages.iter().map(|m| m.text()).collect()
    }

    #[test]
    fn assigns_each_message_to_the_first_commit_at_or_after_it() {
        // Commit A @10:00, commit B @12:00.
        let commits = vec![commit("A", 10), commit("B", 12)];
        // Intentionally out of order to prove sorting.
        let session = session_with(vec![
            user_msg("after-B", Some("2026-01-01T13:00:00Z")),
            user_msg("before-A", Some("2026-01-01T09:00:00Z")),
            user_msg("between", Some("2026-01-01T11:00:00Z")),
            user_msg("no-timestamp", None),
        ]);

        let history = coalesce(PathBuf::from("/proj"), commits, &[session], vec![]);

        assert_eq!(history.groups.len(), 3, "two commits + one pending group");

        // Pre-first-commit work and the first commit's own work group together.
        assert_eq!(history.groups[0].commit.as_ref().unwrap().summary, "A");
        assert_eq!(texts(&history.groups[0]), ["before-A"]);

        assert_eq!(history.groups[1].commit.as_ref().unwrap().summary, "B");
        assert_eq!(texts(&history.groups[1]), ["between"]);

        // Work after the last commit is the trailing, un-committed group.
        assert!(history.groups[2].is_pending());
        assert_eq!(texts(&history.groups[2]), ["after-B"]);

        // Undated messages can't be placed on the timeline.
        assert_eq!(history.undated.len(), 1);
        assert_eq!(history.undated[0].text(), "no-timestamp");
    }

    #[test]
    fn no_commits_puts_everything_in_a_single_pending_group() {
        let session = session_with(vec![user_msg("hi", Some("2026-01-01T09:00:00Z"))]);
        let history = coalesce(PathBuf::from("/proj"), vec![], &[session], vec![]);

        assert_eq!(history.groups.len(), 1);
        assert!(history.groups[0].is_pending());
        assert_eq!(history.message_count(), 1);
    }

    #[test]
    fn commit_boundary_is_inclusive_of_equal_timestamps() {
        // A message exactly at the commit time belongs to that commit.
        let commits = vec![commit("A", 10)];
        let session = session_with(vec![user_msg("at-A", Some("2026-01-01T10:00:00Z"))]);
        let history = coalesce(PathBuf::from("/proj"), commits, &[session], vec![]);

        assert_eq!(texts(&history.groups[0]), ["at-A"]);
        assert_eq!(history.groups.len(), 1, "no trailing pending group");
    }
}
