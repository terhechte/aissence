//! Discovering and loading AI-agent session logs for one or more project
//! folders.
//!
//! Session logs do not live inside the project folder; each agent stores them
//! in its own external location (e.g. Claude Code under `~/.claude/projects/…`).
//! [`normalize_chat_sessions::project_metadata_roots`] tells us where its
//! file-backed agents keep their logs. Agent formats not covered there are
//! loaded by the focused modules in this directory, including SQLite stores,
//! event-sourced JSONL streams, and VS Code workspace storage.
//!
//! Worse, the parsed [`ncs::Session`] does not expose the session's working
//! directory (`metadata.project` is empty in practice), so we cannot scope on
//! it after parsing. Instead we scope by scanning each candidate file for the
//! folder's absolute path: every agent records its working directory near the
//! top of the log (Codex in its `session_meta`, Claude on each message's
//! `cwd`). A trailing path-boundary check keeps `…/aissence` from matching
//! `…/aissence-other`.
//!
//! This is exact for Claude Code and Codex. Gemini identifies projects only by
//! an opaque path hash that it does not store the plaintext path for, so Gemini
//! sessions are matched best-effort (only when they reference the path in their
//! contents).

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;

mod amp;
mod antigravity;
mod codex;
mod common;
mod crush;
mod kimi;
mod opencode;
mod pi_like;
mod vscode_copilot;

/// How many bytes from the start of each candidate file we scan for the folder
/// path. The working directory is always recorded near the top of a log, so a
/// modest cap keeps scanning of large, unrelated logs cheap.
const SCAN_CAP_BYTES: u64 = 256 * 1024;

/// Sessions loaded for a folder, plus any non-fatal load problems.
pub struct LoadedSessions {
    pub sessions: Vec<ncs::Session>,
    /// One entry per file that was discovered but could not be parsed.
    pub warnings: Vec<String>,
}

/// Locate every file-backed session log belonging to `folder`, across the
/// formats supported by `normalize-chat-sessions` (Claude Code, Codex, Gemini,
/// …). OpenCode's SQLite/legacy-JSON stores are handled directly by
/// [`load_sessions`].
///
/// Returns absolute paths to candidate `.jsonl` / `.json` session files. The
/// list is de-duplicated and sorted for deterministic output.
pub fn discover_session_files(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in ncs::project_metadata_roots(folder) {
        collect_session_files(&root, &mut files);
    }
    files.sort();
    files.dedup();
    files
}

/// Recursively collect `.jsonl` / `.json` files under `dir`.
///
/// Recursion matters: some agents nest logs in date- or subagent-keyed
/// subdirectories.
fn collect_session_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("jsonl") | Some("json")
        ) {
            out.push(path);
        }
    }
}

/// Load every session that belongs to `folder`.
///
/// Candidate files are discovered via [`discover_session_files`], scoped to the
/// folder by [`file_references_folder`], then parsed. Files that fail to parse
/// are skipped and recorded as warnings rather than aborting the whole load.
/// Because scoping happens *before* parsing, unrelated logs from other projects
/// neither pollute the result nor generate warning noise.
pub fn load_sessions(folder: &Path) -> LoadedSessions {
    let folders = [folder.to_path_buf()];
    load_sessions_for_folders(&folders)
}

/// Load and merge sessions for several corresponding project folders, such as
/// the main checkout and its linked Git worktrees.
///
/// Some agents store sessions globally and are therefore scanned once per
/// folder. Results are deduplicated by source path, format, and session ID
/// before being returned; this preserves distinct sessions stored in one
/// SQLite database.
pub fn load_sessions_for_folders(folders: &[PathBuf]) -> LoadedSessions {
    let mut sessions = Vec::new();
    let mut session_keys = HashSet::new();
    let mut warnings = Vec::new();
    let mut warning_set = HashSet::new();

    for folder in folders {
        let loaded = load_sessions_for_folder(folder);
        for session in loaded.sessions {
            if session_keys.insert(session_key(&session)) {
                sessions.push(session);
            }
        }
        for warning in loaded.warnings {
            if warning_set.insert(warning.clone()) {
                warnings.push(warning);
            }
        }
    }

    sessions.sort_by(|a, b| {
        a.metadata
            .timestamp
            .cmp(&b.metadata.timestamp)
            .then_with(|| a.path.cmp(&b.path))
    });

    LoadedSessions { sessions, warnings }
}

fn session_key(session: &ncs::Session) -> (PathBuf, String, Option<String>) {
    (
        session
            .path
            .canonicalize()
            .unwrap_or_else(|_| session.path.clone()),
        session.format.clone(),
        session.metadata.session_id.clone(),
    )
}

fn load_sessions_for_folder(folder: &Path) -> LoadedSessions {
    let needles = folder_path_needles(folder);

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();

    for file in discover_session_files(folder) {
        // Current Codex rollouts need the local compatibility parser below.
        // Skipping them here also prevents the same file appearing twice.
        if codex::is_codex_session(&file) {
            continue;
        }
        if !file_references_folder(&file, &needles) {
            continue;
        }
        match ncs::parse_session(&file) {
            Ok(session) => sessions.push(session),
            Err(err) => warnings.push(format!("{}: {err}", file.display())),
        }
    }

    let loaded_opencode = opencode::load_sessions(folder);
    sessions.extend(loaded_opencode.sessions);
    warnings.extend(loaded_opencode.warnings);

    let loaded_kimi = kimi::load_sessions(folder);
    sessions.extend(loaded_kimi.sessions);
    warnings.extend(loaded_kimi.warnings);

    let loaded_pi = pi_like::load_sessions(folder, pi_like::Flavor::Pi);
    sessions.extend(loaded_pi.sessions);
    warnings.extend(loaded_pi.warnings);

    let loaded_omp = pi_like::load_sessions(folder, pi_like::Flavor::Omp);
    sessions.extend(loaded_omp.sessions);
    warnings.extend(loaded_omp.warnings);

    let loaded_amp = amp::load_sessions(folder);
    sessions.extend(loaded_amp.sessions);
    warnings.extend(loaded_amp.warnings);

    let loaded_crush = crush::load_sessions(folder);
    sessions.extend(loaded_crush.sessions);
    warnings.extend(loaded_crush.warnings);

    let loaded_codex = codex::load_sessions(folder);
    sessions.extend(loaded_codex.sessions);
    warnings.extend(loaded_codex.warnings);

    let loaded_antigravity = antigravity::load_sessions(folder);
    sessions.extend(loaded_antigravity.sessions);
    warnings.extend(loaded_antigravity.warnings);

    let loaded_vscode = vscode_copilot::load_sessions(folder);
    sessions.extend(loaded_vscode.sessions);
    warnings.extend(loaded_vscode.warnings);

    LoadedSessions { sessions, warnings }
}

/// Build the search strings that identify `folder` inside a session log.
///
/// Working directories appear in the logs as JSON string values, e.g.
/// `"cwd":"/Users/me/proj"`. We match the absolute path followed by a closing
/// quote (the directory itself) or a slash (a subdirectory), which prevents a
/// sibling like `/Users/me/proj-other` from matching `/Users/me/proj`.
fn folder_path_needles(folder: &Path) -> Vec<String> {
    let abs = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let base = abs.to_string_lossy();
    vec![format!("{base}\""), format!("{base}/")]
}

/// Cheap relevance check: does the head of `path` reference the folder?
///
/// Reads at most [`SCAN_CAP_BYTES`] and looks for any of `needles`. Returns
/// `false` on any I/O error (an unreadable file simply isn't matched).
fn file_references_folder(path: &Path, needles: &[String]) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buf = Vec::new();
    if std::io::BufReader::new(file)
        .take(SCAN_CAP_BYTES)
        .read_to_end(&mut buf)
        .is_err()
    {
        return false;
    }
    let haystack = String::from_utf8_lossy(&buf);
    needles
        .iter()
        .any(|needle| haystack.contains(needle.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aissence-sessions-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loads_sessions_from_linked_scopes_and_deduplicates_repeated_scopes() {
        let main = tmp_dir("worktree");
        let linked = main.join("linked");
        std::fs::create_dir_all(linked.join(".normalize/agent/logs")).unwrap();
        let linked = linked.canonicalize().unwrap();
        let log = linked.join(".normalize/agent/logs/session.jsonl");
        let log_text = format!(
            "{{\"event\":\"session_start\",\"session_id\":\"s1\",\
             \"timestamp\":\"2026-01-01T00:00:00Z\",\"moss_root\":{}}}\n\
             {{\"event\":\"task\",\"user_prompt\":\"prompt from worktree\"}}\n",
            serde_json::to_string(&linked.to_string_lossy()).unwrap()
        );
        std::fs::write(&log, log_text).unwrap();

        assert!(discover_session_files(&linked).contains(&log));
        let raw_log = std::fs::read_to_string(&log).unwrap();
        assert!(raw_log.contains(linked.to_string_lossy().as_ref()));
        assert!(raw_log.contains(&format!("{}\"", linked.display())));
        assert!(file_references_folder(&log, &folder_path_needles(&linked)));

        let folders = [main.clone(), linked.clone(), linked.clone()];
        let loaded = load_sessions_for_folders(&folders);

        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].format, "normalize");
        assert_eq!(
            loaded.sessions[0].metadata.session_id.as_deref(),
            Some("s1")
        );
        assert_eq!(loaded.sessions[0].path, log.canonicalize().unwrap());

        std::fs::remove_dir_all(&main).ok();
    }

    #[test]
    fn session_key_keeps_shared_store_sessions_distinct() {
        let path = PathBuf::from("/tmp/shared-sessions.db");
        let mut first = ncs::Session::new(path.clone(), "opencode");
        first.metadata.session_id = Some("first".into());
        let mut second = ncs::Session::new(path, "opencode");
        second.metadata.session_id = Some("second".into());

        assert_ne!(session_key(&first), session_key(&second));
    }
}
