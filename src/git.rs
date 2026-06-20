//! Reading a folder's commit history via [`gix`].

use std::collections::HashSet;
use std::path::Path;

use chrono::DateTime;

use crate::error::Error;
use crate::model::CommitInfo;

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
        commits.push(to_commit_info(&commit)?);

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

fn to_commit_info(commit: &gix::Commit<'_>) -> Result<CommitInfo, Error> {
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

    Ok(CommitInfo {
        id,
        short_id,
        summary,
        body,
        message: full,
        author_name,
        author_email,
        time: when,
    })
}
