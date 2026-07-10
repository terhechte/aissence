//! Discovering and loading AI-agent session logs for a folder.
//!
//! Session logs do not live inside the project folder; each agent stores them
//! in its own external location (e.g. Claude Code under `~/.claude/projects/…`).
//! [`normalize_chat_sessions::project_metadata_roots`] tells us where the
//! file-backed agents keep their logs — but for some agents (notably Codex)
//! that is a *single global directory* shared by every project, not a
//! per-project one. OpenCode is loaded separately because current releases use
//! a SQLite database (legacy releases used a directory of JSON files).
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

use std::io::Read;
use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;

mod opencode;

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
    let needles = folder_path_needles(folder);

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();

    for file in discover_session_files(folder) {
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
    needles.iter().any(|needle| haystack.contains(needle.as_str()))
}
