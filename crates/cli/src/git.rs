//! Reading a folder's commit history via [`gix`].

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::model::{CommitFileChange, CommitFileStatus, CommitInfo, CommitStats};

/// Optional project identity captured in the `.aissence` cache.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// The name of the repository's working-tree directory, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The configured remote in `host/path` form, without credentials or a
    /// protocol prefix (for example, `github.com/org/repo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

/// Best-effort project identity for the repository containing `folder`.
///
/// This intentionally returns optional fields rather than an error: a git
/// repository may have no working tree, no configured remote, or a remote that
/// cannot be parsed as a host/path pair.
pub fn project_metadata(folder: &Path) -> Option<ProjectMetadata> {
    let repo = gix::discover(folder).ok()?;
    let name = repo
        .workdir()
        .or(Some(folder))
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let repository = remote_repository(&repo);

    (name.is_some() || repository.is_some()).then_some(ProjectMetadata { name, repository })
}

/// Return the corresponding analysis path in every checkout of the repository
/// containing `folder`.
///
/// The main worktree and all linked worktrees are included. If `folder` is a
/// subdirectory of a worktree, the same relative subdirectory is returned for
/// every checkout. This lets session loaders use the actual path recorded by
/// an agent while keeping `analyze()` scoped to the caller's original folder.
///
/// Missing or inaccessible linked worktrees are ignored. If the repository
/// cannot be discovered, the canonicalized input folder is returned so callers
/// retain the normal single-folder behavior.
pub fn worktree_scopes(folder: &Path) -> Vec<PathBuf> {
    let input = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let Ok(repo) = gix::discover(folder) else {
        return vec![input];
    };

    let current_root = repo
        .workdir()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    let relative = current_root
        .as_deref()
        .and_then(|root| input.strip_prefix(root).ok())
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let mut roots = Vec::new();
    if let Some(workdir) = repo.workdir() {
        roots.push(workdir.to_path_buf());
    }
    if let Ok(main_repo) = repo.main_repo()
        && let Some(workdir) = main_repo.workdir()
    {
        roots.push(workdir.to_path_buf());
    }
    if let Ok(worktrees) = repo.worktrees() {
        for worktree in worktrees {
            if let Ok(workdir) = worktree.base() {
                roots.push(workdir);
            }
        }
    }

    let scopes = roots
        .into_iter()
        .map(|root| root.canonicalize().unwrap_or(root))
        .map(|root| root.join(&relative))
        .filter(|path| path.is_dir())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    if scopes.is_empty() {
        vec![input]
    } else {
        scopes
    }
}

fn remote_repository(repo: &gix::Repository) -> Option<String> {
    let remote = repo.find_remote("origin").ok().or_else(|| {
        repo.find_default_remote(gix::remote::Direction::Fetch)
            .and_then(Result::ok)
    })?;
    let url = remote.url(gix::remote::Direction::Fetch)?;
    let host = url.host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }
    let path = String::from_utf8_lossy(url.path.as_ref())
        .trim_matches('/')
        .to_owned();
    let path = path
        .strip_suffix(".git")
        .unwrap_or(&path)
        .trim_matches('/')
        .to_owned();
    if path.is_empty() {
        return None;
    }

    let host = match url.port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    Some(format!("{host}/{path}"))
}

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

    #[test]
    fn project_metadata_uses_worktree_name_and_remote_host_path() {
        let dir = tmp_dir("project-metadata");
        git(&dir, &["init", "-q"]);
        git(
            &dir,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:owner/aissence.git",
            ],
        );

        let name = dir.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(
            project_metadata(&dir),
            Some(ProjectMetadata {
                name: Some(name),
                repository: Some("github.com/owner/aissence".into()),
            })
        );

        git(
            &dir,
            &[
                "remote",
                "set-url",
                "origin",
                "https://git.example.test/team/aissence.git",
            ],
        );
        assert_eq!(
            project_metadata(&dir).and_then(|project| project.repository),
            Some("git.example.test/team/aissence".into())
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn worktree_scopes_include_main_linked_and_corresponding_subdirectories() {
        let dir = tmp_dir("worktree-scopes");
        let linked = dir.with_file_name(format!(
            "{}-linked",
            dir.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&linked);

        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.name", "Tester"]);
        git(&dir, &["config", "user.email", "tester@example.com"]);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "Initial source"]);
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                linked.to_str().unwrap(),
            ],
        );

        let main = dir.canonicalize().unwrap();
        let linked = linked.canonicalize().unwrap();
        let scopes = worktree_scopes(&dir);
        assert!(scopes.contains(&main));
        assert!(scopes.contains(&linked));

        let nested_scopes = worktree_scopes(&dir.join("src"));
        assert!(nested_scopes.contains(&main.join("src")));
        assert!(nested_scopes.contains(&linked.join("src")));

        git(
            &dir,
            &["worktree", "remove", "--force", linked.to_str().unwrap()],
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
