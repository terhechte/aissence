//! Reading a folder's commit history via [`gix`].

use std::collections::HashSet;
use std::path::Path;

use chrono::DateTime;

use crate::error::Error;
use crate::model::{CommitFileChange, CommitFileStatus, CommitInfo, CommitStats};

/// Return every commit reachable from `HEAD` for the repository containing
/// `folder`, sorted oldest-first by committer time.
///
/// If the path is inside a git repository that has no commits yet (an unborn
/// `HEAD`), an empty list is returned rather than an error — the chat messages
/// can still be surfaced as a single "pending" group by the caller.
pub fn commit_log(folder: &Path) -> Result<Vec<CommitInfo>, Error> {
    let repo = gix::discover(folder).map_err(|e| Error::Git(e.to_string()))?;

    // An unborn HEAD (fresh repo, no commits) is not an error for us.
    let head = match repo.head_commit() {
        Ok(commit) => commit,
        Err(_) => return Ok(Vec::new()),
    };

    // Walk the full ancestry. We collect every reachable commit (following all
    // parents, so merges are included) and sort by time afterwards, which keeps
    // us independent of any particular traversal ordering.
    let mut seen: HashSet<gix::ObjectId> = HashSet::new();
    let mut stack = vec![head];
    let mut commits = Vec::new();

    while let Some(commit) = stack.pop() {
        let oid = commit.id().detach();
        if !seen.insert(oid) {
            continue;
        }
        commits.push(to_commit_info(&repo, &commit)?);

        let parents: Vec<gix::ObjectId> = commit.parent_ids().map(|id| id.detach()).collect();
        for parent in parents {
            if let Ok(parent_commit) = repo.find_commit(parent) {
                stack.push(parent_commit);
            }
        }
    }

    commits.sort_by_key(|c| c.time);
    Ok(commits)
}

/// Return the unified diff (patch text) introduced by `commit_id` in the
/// repository containing `folder`.
///
/// This shells out to the `git` CLI (`git show`) — the most reliable way to
/// render a textual patch. The commit *log* uses `gix`; only patch generation
/// uses the CLI. Feed the result to [`crate::DiffView`].
pub fn commit_patch(folder: &Path, commit_id: &str) -> Result<String, Error> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["show", "--no-color", "--format=", commit_id])
        .output()
        .map_err(|e| Error::Git(format!("running git show: {e}")))?;

    if !output.status.success() {
        return Err(Error::Git(format!(
            "git show {commit_id}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn to_commit_info(repo: &gix::Repository, commit: &gix::Commit<'_>) -> Result<CommitInfo, Error> {
    let map_git = |e: &dyn std::fmt::Display| Error::Git(e.to_string());

    let id = commit.id().detach().to_hex().to_string();
    let short_id = match commit.short_id() {
        Ok(prefix) => prefix.to_string(),
        Err(_) => id.chars().take(7).collect(),
    };

    let time = commit.time().map_err(|e| map_git(&e))?;
    let when = DateTime::from_timestamp(time.seconds, 0).unwrap_or_default();

    let author = commit.author().map_err(|e| map_git(&e))?;
    let author_name = author.name.to_string();
    let author_email = author.email.to_string();

    let message = commit.message().map_err(|e| map_git(&e))?;
    let summary = message.summary().to_string();
    let body = message.body.map(|b| b.to_string());
    let full = commit.message_raw_sloppy().to_string();
    let (stats, files) = commit_diff_metadata(repo, commit)?;

    Ok(CommitInfo {
        id,
        short_id,
        summary,
        body,
        message: full,
        author_name,
        author_email,
        time: when,
        stats,
        files,
    })
}

/// Calculate authoritative statistics and changed paths against the first
/// parent. Root commits are compared with the empty tree.
fn commit_diff_metadata(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
) -> Result<(CommitStats, Vec<CommitFileChange>), Error> {
    let map_git = |e: &dyn std::fmt::Display| Error::Git(e.to_string());

    let current_tree = commit.tree().map_err(|e| map_git(&e))?;
    let previous_tree = match commit.parent_ids().next() {
        Some(parent_id) => repo
            .find_commit(parent_id.detach())
            .map_err(|e| map_git(&e))?
            .tree()
            .map_err(|e| map_git(&e))?,
        None => repo.empty_tree(),
    };

    let mut platform = previous_tree.changes().map_err(|e| map_git(&e))?;
    platform.options(|options| {
        // Make rename handling deterministic rather than dependent on the
        // repository's `diff.renames` configuration.
        options.track_rewrites(Some(Default::default()));
    });
    let lines = platform.stats(&current_tree).map_err(|e| map_git(&e))?;

    // `gix` line stats intentionally omit binary files from `files_changed`.
    // Count changed non-tree entries separately so binary files and submodules
    // are represented in the authoritative file total.
    let mut options = gix::diff::Options::default();
    options.track_rewrites(Some(Default::default()));
    let changes = repo
        .diff_tree_to_tree(Some(&previous_tree), Some(&current_tree), Some(options))
        .map_err(|e| map_git(&e))?;
    let files = changes
        .iter()
        .filter_map(|change| {
            use gix::object::tree::diff::ChangeDetached;
            match change {
                ChangeDetached::Addition {
                    location,
                    entry_mode,
                    ..
                } if !entry_mode.is_tree() => Some(CommitFileChange {
                    status: CommitFileStatus::Added,
                    path: String::from_utf8_lossy(location.as_ref()).into_owned(),
                    previous_path: None,
                }),
                ChangeDetached::Deletion {
                    location,
                    entry_mode,
                    ..
                } if !entry_mode.is_tree() => Some(CommitFileChange {
                    status: CommitFileStatus::Deleted,
                    path: String::from_utf8_lossy(location.as_ref()).into_owned(),
                    previous_path: None,
                }),
                ChangeDetached::Modification {
                    location,
                    entry_mode,
                    ..
                } if !entry_mode.is_tree() => Some(CommitFileChange {
                    status: CommitFileStatus::Modified,
                    path: String::from_utf8_lossy(location.as_ref()).into_owned(),
                    previous_path: None,
                }),
                ChangeDetached::Rewrite {
                    source_location,
                    location,
                    entry_mode,
                    copy,
                    ..
                } if !entry_mode.is_tree() => Some(CommitFileChange {
                    status: if *copy {
                        CommitFileStatus::Copied
                    } else {
                        CommitFileStatus::Renamed
                    },
                    path: String::from_utf8_lossy(location.as_ref()).into_owned(),
                    previous_path: Some(
                        String::from_utf8_lossy(source_location.as_ref()).into_owned(),
                    ),
                }),
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    Ok((
        CommitStats {
            files_changed: files.len() as u64,
            lines_added: lines.lines_added,
            lines_deleted: lines.lines_removed,
        },
        files,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aissence-git-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn commit_log_reports_tree_based_text_and_binary_stats() {
        let dir = tmp_dir("stats");
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.name", "Tester"]);
        git(&dir, &["config", "user.email", "tester@example.com"]);

        std::fs::write(dir.join("notes.txt"), "one\n").unwrap();
        git(&dir, &["add", "notes.txt"]);
        git(&dir, &["commit", "-qm", "Initial notes"]);

        std::fs::write(dir.join("notes.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("image.bin"), [0, 1, 2, 3]).unwrap();
        git(&dir, &["add", "notes.txt", "image.bin"]);
        git(&dir, &["commit", "-qm", "Add a line and binary"]);

        git(&dir, &["mv", "notes.txt", "renamed.txt"]);
        git(&dir, &["commit", "-qm", "Rename notes"]);

        let commits = commit_log(&dir).unwrap();
        assert_eq!(commits.len(), 3);
        let initial = commits
            .iter()
            .find(|commit| commit.summary == "Initial notes")
            .unwrap();
        assert_eq!(
            initial.stats,
            CommitStats {
                files_changed: 1,
                lines_added: 1,
                lines_deleted: 0,
            }
        );
        assert_eq!(
            initial.files,
            vec![CommitFileChange {
                status: CommitFileStatus::Added,
                path: "notes.txt".into(),
                previous_path: None,
            }]
        );
        let second = commits
            .iter()
            .find(|commit| commit.summary == "Add a line and binary")
            .unwrap();
        assert_eq!(
            second.stats,
            CommitStats {
                files_changed: 2,
                lines_added: 1,
                lines_deleted: 0,
            }
        );
        assert!(second.files.contains(&CommitFileChange {
            status: CommitFileStatus::Modified,
            path: "notes.txt".into(),
            previous_path: None,
        }));
        assert!(second.files.contains(&CommitFileChange {
            status: CommitFileStatus::Added,
            path: "image.bin".into(),
            previous_path: None,
        }));
        let renamed = commits
            .iter()
            .find(|commit| commit.summary == "Rename notes")
            .unwrap();
        assert_eq!(
            renamed.stats,
            CommitStats {
                files_changed: 1,
                lines_added: 0,
                lines_deleted: 0,
            }
        );
        assert_eq!(
            renamed.files,
            vec![CommitFileChange {
                status: CommitFileStatus::Renamed,
                path: "renamed.txt".into(),
                previous_path: Some("notes.txt".into()),
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
