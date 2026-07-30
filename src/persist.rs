//! Persisting commit summaries and their supporting git metadata to
//! `<folder>/.aissence` so the app can resume where it left off instead of
//! re-summarizing on every launch.
//!
//! Summaries are keyed by full commit id — a commit (and the conversation that
//! led to it) is immutable, so a cached summary stays valid forever. A content
//! [`signature`] of the group's messages is stored alongside, so the rare case
//! of sessions being backfilled into an old commit's window invalidates the
//! stale entry.
//!
//! The cache is written atomically (temp file + rename) after every summary, so
//! quitting mid-run loses at most the in-flight commit.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{CommitGroup, CommitInfo, CommitStats, CommitSummary};

/// File name (inside the analyzed folder) where summaries are cached.
pub const CACHE_FILE: &str = ".aissence";

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    signature: u64,
    /// Git metadata copied into the cache so consumers do not need repository
    /// access for common commit-list data. Optional for old version-1 files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stats: Option<CommitStats>,
    summary: CommitSummary,
}

#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    version: u32,
    /// Keyed by full commit hex id.
    commits: BTreeMap<String, CacheEntry>,
    /// Commit ids the user has chosen not to summarize.
    #[serde(default)]
    ignored: BTreeSet<String>,
}

/// In-memory + on-disk store of commit summaries, git metadata, and ignore
/// flags for one folder.
pub struct SummaryStore {
    path: PathBuf,
    commits: BTreeMap<String, CacheEntry>,
    ignored: BTreeSet<String>,
}

impl SummaryStore {
    /// Load the store for `folder`. Returns an empty store when the file is
    /// missing, unreadable, corrupt, or written by a different version.
    pub fn load(folder: &Path) -> Self {
        Self::load_path(folder.join(CACHE_FILE))
    }

    /// Load a store from an explicitly named cache file.
    ///
    /// This is useful for side-by-side model runs that must not replace the
    /// analyzed project's regular `.aissence` cache.
    pub fn load_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (commits, ignored) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<CacheFile>(&text).ok())
            .filter(|cache| cache.version == VERSION)
            .map(|cache| (cache.commits, cache.ignored))
            .unwrap_or_default();
        Self {
            path,
            commits,
            ignored,
        }
    }

    /// Create an empty store that will be written to an explicitly named file.
    pub fn empty_at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            commits: BTreeMap::new(),
            ignored: BTreeSet::new(),
        }
    }

    /// Whether the user marked `commit_id` as ignored.
    pub fn is_ignored(&self, commit_id: &str) -> bool {
        self.ignored.contains(commit_id)
    }

    /// Mark `commit_id` ignored (or not) and persist if the flag changed.
    pub fn set_ignored(&mut self, commit_id: &str, ignored: bool) -> std::io::Result<()> {
        let changed = if ignored {
            self.ignored.insert(commit_id.to_string())
        } else {
            self.ignored.remove(commit_id)
        };
        if changed { self.save() } else { Ok(()) }
    }

    /// Number of cached commit summaries.
    pub fn len(&self) -> usize {
        self.commits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    /// Return a cached summary for `commit_id`, but only if its stored signature
    /// matches `signature` (i.e. the underlying messages are unchanged).
    pub fn get(&self, commit_id: &str, signature: u64) -> Option<&CommitSummary> {
        self.commits
            .get(commit_id)
            .filter(|entry| entry.signature == signature)
            .map(|entry| &entry.summary)
    }

    /// Replace cached summaries in place (no new summarization) and persist once.
    ///
    /// Each tuple is `(commit_id, signature, summary)`; an entry is updated only
    /// when a cached one with a matching signature exists. Used to backfill
    /// session-derived data such as token counts and retained source text onto
    /// older summaries, in a single write rather than one per commit.
    pub fn backfill_summaries(
        &mut self,
        updates: Vec<(String, u64, CommitSummary)>,
    ) -> std::io::Result<()> {
        let mut changed = false;
        for (id, signature, summary) in updates {
            if let Some(entry) = self.commits.get_mut(&id)
                && entry.signature == signature
            {
                entry.summary = summary;
                changed = true;
            }
        }
        if changed { self.save() } else { Ok(()) }
    }

    /// Populate or refresh immutable git metadata on existing cached entries
    /// and persist once. This upgrades older version-1 files without requiring
    /// their conversations to be summarized again.
    pub fn backfill_commit_metadata<'a>(
        &mut self,
        commits: impl IntoIterator<Item = &'a CommitInfo>,
    ) -> std::io::Result<()> {
        let mut changed = false;
        for commit in commits {
            if let Some(entry) = self.commits.get_mut(&commit.id) {
                let title = Some(commit.summary.clone());
                let date = Some(commit.time);
                let stats = Some(commit.stats);
                if entry.title != title || entry.date != date || entry.stats != stats {
                    entry.title = title;
                    entry.date = date;
                    entry.stats = stats;
                    changed = true;
                }
            }
        }
        if changed { self.save() } else { Ok(()) }
    }

    /// Insert (or replace) a summary and persist the whole store to disk.
    pub fn insert_and_save(
        &mut self,
        commit: &CommitInfo,
        signature: u64,
        summary: CommitSummary,
    ) -> std::io::Result<()> {
        self.commits.insert(
            commit.id.clone(),
            CacheEntry {
                signature,
                title: Some(commit.summary.clone()),
                date: Some(commit.time),
                stats: Some(commit.stats),
                summary,
            },
        );
        self.save()
    }

    /// Replace this store with all summarized committed groups and persist it
    /// in a single atomic write.
    ///
    /// The trailing uncommitted group has no stable commit id and therefore
    /// cannot be represented by the version-1 cache format.
    pub fn replace_from_groups(&mut self, groups: &[CommitGroup]) -> std::io::Result<()> {
        self.commits.clear();
        self.ignored.clear();
        for group in groups {
            let (Some(commit), Some(summary)) = (&group.commit, &group.summary) else {
                continue;
            };
            self.commits.insert(
                commit.id.clone(),
                CacheEntry {
                    signature: signature(group),
                    title: Some(commit.summary.clone()),
                    date: Some(commit.time),
                    stats: Some(commit.stats),
                    summary: summary.clone(),
                },
            );
        }
        self.save()
    }

    /// Atomically write the store (temp file + rename) so a crash mid-write
    /// can't corrupt the cache.
    fn save(&self) -> std::io::Result<()> {
        let file = CacheFile {
            version: VERSION,
            commits: self.commits.clone(),
            ignored: self.ignored.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        let file_name = self
            .path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(CACHE_FILE));
        let mut tmp_name = file_name.to_os_string();
        tmp_name.push(".tmp");
        let tmp = self.path.with_file_name(tmp_name);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// A content signature of a commit group's messages. A cached summary is reused
/// only when this matches, so changed/backfilled sessions invalidate it.
pub fn signature(group: &CommitGroup) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    group.messages.len().hash(&mut hasher);
    for message in &group.messages {
        role_index(message.role).hash(&mut hasher);
        message.content.len().hash(&mut hasher);
        message.text().hash(&mut hasher);
        message
            .timestamp
            .map(|t| t.timestamp_millis())
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn role_index(role: crate::ncs::Role) -> u8 {
    match role {
        crate::ncs::Role::User => 0,
        crate::ncs::Role::Assistant => 1,
        crate::ncs::Role::System => 2,
        crate::ncs::Role::Tool => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AgentUsage, CommitInfo, CommitStats, CommitSummary, MessageKind, MessageSummary, TokenCount,
    };
    use chrono::TimeZone;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aissence-persist-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> CommitSummary {
        CommitSummary {
            user_requested: "did things".into(),
            ai_answered: "explained".into(),
            ai_implemented: "edited code".into(),
            messages: vec![MessageSummary {
                kind: MessageKind::UserWrote,
                summary: Some("A greeting.".into()),
                content: Some("hello".into()),
                changes: vec![],
                timestamp: None,
                tokens: None,
            }],
            agents: vec![AgentUsage {
                provider: "Codex".into(),
                model: Some("gpt-test".into()),
                tokens: Some(TokenCount {
                    input: 10,
                    output: 4,
                    cache_read: Some(2),
                    cache_create: None,
                }),
            }],
            meta: "an overview".into(),
            tokens: None,
        }
    }

    fn commit() -> CommitInfo {
        CommitInfo {
            id: "abc123".into(),
            short_id: "abc123".into(),
            summary: "Add useful metadata".into(),
            body: None,
            message: "Add useful metadata".into(),
            author_name: "tester".into(),
            author_email: "t@example.com".into(),
            time: chrono::Utc
                .with_ymd_and_hms(2026, 7, 29, 10, 30, 0)
                .unwrap(),
            stats: CommitStats {
                files_changed: 3,
                lines_added: 21,
                lines_deleted: 4,
            },
            files: vec![],
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tmp_dir("rt");
        {
            let mut store = SummaryStore::load(&dir);
            store.insert_and_save(&commit(), 42, sample()).unwrap();
        }
        // The file actually lands in the folder.
        assert!(dir.join(CACHE_FILE).exists());
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CACHE_FILE)).unwrap()).unwrap();
        let entry = &json["commits"]["abc123"];
        assert_eq!(entry["title"], "Add useful metadata");
        assert_eq!(entry["date"], "2026-07-29T10:30:00Z");
        assert_eq!(entry["stats"]["files_changed"], 3);
        assert_eq!(entry["stats"]["lines_added"], 21);
        assert_eq!(entry["stats"]["lines_deleted"], 4);
        assert_eq!(entry["summary"]["agents"][0]["model"], "gpt-test");
        assert_eq!(entry["summary"]["agents"][0]["tokens"]["input"], 10);
        assert_eq!(entry["summary"]["agents"][0]["tokens"]["output"], 4);
        assert_eq!(entry["summary"]["messages"][0]["summary"], "A greeting.");

        let store = SummaryStore::load(&dir);
        let got = store.get("abc123", 42).expect("cached entry");
        assert_eq!(got.user_requested, "did things");
        assert_eq!(got.meta, "an overview");
        assert_eq!(got.messages.len(), 1);
        assert_eq!(got.messages[0].content.as_deref(), Some("hello"));
        assert_eq!(got.agents[0].model.as_deref(), Some("gpt-test"));
        assert_eq!(got.agents[0].tokens.map(|tokens| tokens.input), Some(10));

        // Signature mismatch and unknown id both miss.
        assert!(store.get("abc123", 99).is_none());
        assert!(store.get("missing", 42).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writes_a_complete_history_to_an_explicit_cache_path() {
        let dir = tmp_dir("named-cache");
        let path = dir.join("vendor-model.aissence");
        let group = CommitGroup {
            commit: Some(commit()),
            messages: vec![],
            summary: Some(sample()),
        };
        let expected_signature = signature(&group);

        let mut store = SummaryStore::empty_at(&path);
        store.replace_from_groups(&[group]).unwrap();

        assert!(path.exists());
        assert!(!dir.join(CACHE_FILE).exists());
        assert!(
            SummaryStore::load_path(&path)
                .get("abc123", expected_signature)
                .is_some()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backfills_git_metadata_onto_an_older_entry() {
        let dir = tmp_dir("old-metadata");
        let json = serde_json::json!({
            "version": 1,
            "commits": {
                "abc123": {
                    "signature": 42,
                    "summary": sample()
                }
            }
        });
        std::fs::write(
            dir.join(CACHE_FILE),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();

        let mut store = SummaryStore::load(&dir);
        store.backfill_commit_metadata([&commit()]).unwrap();

        let upgraded: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CACHE_FILE)).unwrap()).unwrap();
        let entry = &upgraded["commits"]["abc123"];
        assert_eq!(entry["title"], "Add useful metadata");
        assert_eq!(entry["date"], "2026-07-29T10:30:00Z");
        assert_eq!(entry["stats"]["files_changed"], 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn older_messages_without_source_content_still_load() {
        let dir = tmp_dir("old-content");
        let json = serde_json::json!({
            "version": 1,
            "commits": {
                "abc123": {
                    "signature": 42,
                    "summary": {
                        "user_requested": "asked",
                        "ai_answered": "",
                        "ai_implemented": "",
                        "agents": [{
                            "provider": "Codex",
                            "model": "gpt-old"
                        }],
                        "messages": [{
                            "kind": "UserWrote",
                            "summary": "asked",
                            "changes": [],
                            "timestamp": null
                        }]
                    }
                }
            },
            "ignored": []
        });
        std::fs::write(
            dir.join(CACHE_FILE),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();

        let store = SummaryStore::load(&dir);
        let summary = store.get("abc123", 42).unwrap();
        let message = &summary.messages[0];
        assert_eq!(message.summary.as_deref(), Some("asked"));
        assert_eq!(message.content, None);
        assert_eq!(summary.agents[0].tokens, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn assistant_entries_do_not_emit_a_summary() {
        let message = MessageSummary {
            kind: MessageKind::AiWrote,
            summary: None,
            content: Some("complete assistant reply".into()),
            changes: vec![],
            timestamp: None,
            tokens: None,
        };

        let json = serde_json::to_value(message).unwrap();
        assert!(json.get("summary").is_none());
        assert_eq!(json["content"], "complete assistant reply");
    }

    #[test]
    fn ignore_flag_persists_and_toggles() {
        let dir = tmp_dir("ignore");
        {
            let mut store = SummaryStore::load(&dir);
            store.set_ignored("deadbeef", true).unwrap();
        }
        assert!(SummaryStore::load(&dir).is_ignored("deadbeef"));
        {
            let mut store = SummaryStore::load(&dir);
            store.set_ignored("deadbeef", false).unwrap();
        }
        assert!(!SummaryStore::load(&dir).is_ignored("deadbeef"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let dir = tmp_dir("missing");
        let store = SummaryStore::load(&dir);
        assert!(store.is_empty());
        assert!(store.get("x", 0).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
