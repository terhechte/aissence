//! Persisting commit summaries to `<folder>/.aissence` so the app can resume
//! where it left off instead of re-summarizing on every launch.
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

use crate::model::{CommitGroup, CommitSummary};

/// File name (inside the analyzed folder) where summaries are cached.
pub const CACHE_FILE: &str = ".aissence";

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    signature: u64,
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

/// In-memory + on-disk store of commit summaries (and ignore flags) for one
/// folder.
pub struct SummaryStore {
    path: PathBuf,
    commits: BTreeMap<String, CacheEntry>,
    ignored: BTreeSet<String>,
}

impl SummaryStore {
    /// Load the store for `folder`. Returns an empty store when the file is
    /// missing, unreadable, corrupt, or written by a different version.
    pub fn load(folder: &Path) -> Self {
        let path = folder.join(CACHE_FILE);
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

    /// Insert (or replace) a summary and persist the whole store to disk.
    pub fn insert_and_save(
        &mut self,
        commit_id: &str,
        signature: u64,
        summary: CommitSummary,
    ) -> std::io::Result<()> {
        self.commits
            .insert(commit_id.to_string(), CacheEntry { signature, summary });
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
        let tmp = self.path.with_file_name(format!("{CACHE_FILE}.tmp"));
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
    use crate::model::{CommitSummary, MessageKind, MessageSummary};

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
                summary: "hello".into(),
                changes: vec![],
                timestamp: None,
            }],
            agents: vec![],
            meta: "an overview".into(),
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tmp_dir("rt");
        {
            let mut store = SummaryStore::load(&dir);
            store.insert_and_save("abc123", 42, sample()).unwrap();
        }
        // The file actually lands in the folder.
        assert!(dir.join(CACHE_FILE).exists());

        let store = SummaryStore::load(&dir);
        let got = store.get("abc123", 42).expect("cached entry");
        assert_eq!(got.user_requested, "did things");
        assert_eq!(got.meta, "an overview");
        assert_eq!(got.messages.len(), 1);

        // Signature mismatch and unknown id both miss.
        assert!(store.get("abc123", 99).is_none());
        assert!(store.get("missing", 42).is_none());

        std::fs::remove_dir_all(&dir).ok();
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
