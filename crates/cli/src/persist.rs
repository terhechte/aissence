//! Atomic persistence for the version-2 `.aissence` interchange document.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::document::{AissenceCommit, AissenceDocument, AissenceProject};
use crate::model::{CommitGroup, CommitSummary};

pub const CACHE_FILE: &str = ".aissence";

pub fn read_document(path: &Path) -> Result<AissenceDocument, DocumentError> {
    let text = std::fs::read_to_string(path)?;
    let document: AissenceDocument = serde_json::from_str(&text)?;
    document
        .validate_version()
        .map_err(DocumentError::Invalid)?;
    Ok(document)
}

pub fn write_document(path: &Path, document: &AissenceDocument) -> Result<(), DocumentError> {
    document
        .validate_version()
        .map_err(DocumentError::Invalid)?;
    let mut sanitized = document.clone();
    crate::Redactor::global().redact_document(&mut sanitized);
    let json = serde_json::to_string_pretty(&sanitized)?;
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new(CACHE_FILE));
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(".tmp");
    let temporary = path.with_file_name(temporary_name);
    std::fs::write(&temporary, json)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid .aissence JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid .aissence document: {0}")]
    Invalid(String),
}

/// In-memory and on-disk store for one project's source data and summaries.
pub struct SummaryStore {
    path: PathBuf,
    project_folder: Option<PathBuf>,
    document: AissenceDocument,
}

impl SummaryStore {
    pub fn load(folder: &Path) -> Self {
        Self::load_path_for_folder(folder.join(CACHE_FILE), folder)
    }

    pub fn load_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let project_folder = path.parent().map(Path::to_path_buf);
        Self::load_path_with_folder(path, project_folder.as_deref())
    }

    pub fn load_path_for_folder(path: impl Into<PathBuf>, folder: &Path) -> Self {
        Self::load_path_with_folder(path.into(), Some(folder))
    }

    fn load_path_with_folder(path: PathBuf, project_folder: Option<&Path>) -> Self {
        let metadata = project_folder.and_then(crate::git::project_metadata);
        let document = read_document(&path)
            .unwrap_or_else(|_| AissenceDocument::empty(AissenceProject::new(metadata.clone())));
        let mut store = Self {
            path,
            project_folder: project_folder.map(Path::to_path_buf),
            document,
        };
        store.document.project.refresh_metadata(metadata);
        store
    }

    pub fn empty_at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let project_folder = path.parent().map(Path::to_path_buf);
        Self::empty_at_with_folder(path, project_folder.as_deref())
    }

    pub fn empty_at_for_folder(path: impl Into<PathBuf>, folder: &Path) -> Self {
        Self::empty_at_with_folder(path.into(), Some(folder))
    }

    fn empty_at_with_folder(path: PathBuf, project_folder: Option<&Path>) -> Self {
        let project = AissenceProject::new(project_folder.and_then(crate::git::project_metadata));
        Self {
            path,
            project_folder: project_folder.map(Path::to_path_buf),
            document: AissenceDocument::empty(project),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn document(&self) -> &AissenceDocument {
        &self.document
    }

    pub fn project(&self) -> &AissenceProject {
        &self.document.project
    }

    pub fn len(&self) -> usize {
        self.document.commits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.document.commits.is_empty()
    }

    pub fn is_ignored(&self, commit_id: &str) -> bool {
        self.document.ignored.contains(commit_id)
    }

    pub fn set_ignored(&mut self, commit_id: &str, ignored: bool) -> std::io::Result<()> {
        let changed = if ignored {
            self.document.ignored.insert(commit_id.to_string())
        } else {
            self.document.ignored.remove(commit_id)
        };
        if changed { self.save_io() } else { Ok(()) }
    }

    /// Return a cached summary only when the stable source digest still
    /// matches the current commit group.
    pub fn get_for_group(&self, group: &CommitGroup) -> Option<&CommitSummary> {
        let current = AissenceCommit::from_group(group)?;
        self.document
            .commits
            .iter()
            .find(|stored| {
                stored.commit_hash == current.commit_hash
                    && stored.source_digest == current.source_digest
            })
            .and_then(|stored| stored.summary.as_ref())
    }

    /// Refresh every committed source entry while preserving summaries whose
    /// source digest is unchanged.
    pub fn sync_groups(&mut self, groups: &[CommitGroup]) -> std::io::Result<()> {
        let mut commits = Vec::new();
        for group in groups {
            let Some(mut current) = AissenceCommit::from_group(group) else {
                continue;
            };
            if current.summary.is_none()
                && let Some(existing) = self.document.commits.iter().find(|existing| {
                    existing.commit_hash == current.commit_hash
                        && existing.source_digest == current.source_digest
                })
            {
                current.summary = existing.summary.clone();
            }
            commits.push(current);
        }
        self.document.commits = commits;
        self.save_io()
    }

    pub fn insert_and_save(
        &mut self,
        group: &CommitGroup,
        summary: CommitSummary,
    ) -> std::io::Result<()> {
        let Some(mut current) = AissenceCommit::from_group(group) else {
            return Ok(());
        };
        current.summary = Some(summary);
        match self
            .document
            .commits
            .iter_mut()
            .find(|stored| stored.commit_hash == current.commit_hash)
        {
            Some(stored) => *stored = current,
            None => self.document.commits.push(current),
        }
        self.save_io()
    }

    pub fn replace_from_groups(&mut self, groups: &[CommitGroup]) -> std::io::Result<()> {
        self.document.ignored.clear();
        self.sync_groups(groups)
    }

    pub fn merge_summary_delta(
        &mut self,
        delta: &AissenceDocument,
    ) -> Result<usize, DocumentError> {
        delta.validate_version().map_err(DocumentError::Invalid)?;
        if delta.project.source_id != self.document.project.source_id {
            return Err(DocumentError::Invalid(
                "summary delta belongs to a different project".into(),
            ));
        }
        let merged = self.document.merge_summaries(delta);
        write_document(&self.path, &self.document)?;
        Ok(merged)
    }

    pub fn delta(&self, hashes: &BTreeSet<String>) -> AissenceDocument {
        self.document.delta(hashes)
    }

    fn save_io(&mut self) -> std::io::Result<()> {
        if let Some(metadata) = self
            .project_folder
            .as_deref()
            .and_then(crate::git::project_metadata)
        {
            self.document.project.refresh_metadata(Some(metadata));
        }
        write_document(&self.path, &self.document).map_err(|error| match error {
            DocumentError::Io(error) => error,
            other => std::io::Error::other(other),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CommitInfo, CommitStats};
    use chrono::{TimeZone, Utc};

    fn directory(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("aissence-v2-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn group() -> CommitGroup {
        CommitGroup {
            commit: Some(CommitInfo {
                id: "a".repeat(40),
                short_id: "aaaaaaa".into(),
                summary: "A commit".into(),
                body: None,
                message: "A commit".into(),
                author_name: "Not persisted".into(),
                author_email: "secret@example.test".into(),
                time: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
                stats: CommitStats::default(),
                files: Vec::new(),
            }),
            messages: Vec::new(),
            summary: Some(CommitSummary {
                public: crate::model::PublicStory {
                    summary: "Done".into(),
                    beats: Vec::new(),
                },
                ..Default::default()
            }),
        }
    }

    #[test]
    fn writes_version_two_and_omits_author_identity() {
        let dir = directory("write");
        let mut store = SummaryStore::load(&dir);
        store.replace_from_groups(&[group()]).unwrap();
        let text = std::fs::read_to_string(dir.join(CACHE_FILE)).unwrap();
        let document: AissenceDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(document.version, crate::document::DOCUMENT_VERSION);
        assert_eq!(document.commits.len(), 1);
        assert!(!text.contains("secret@example.test"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rejects_a_summary_delta_for_another_project() {
        let dir = directory("delta");
        let mut store = SummaryStore::load(&dir);
        store.replace_from_groups(&[group()]).unwrap();
        let other = AissenceDocument::empty(AissenceProject::new(None));
        assert!(store.merge_summary_delta(&other).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
