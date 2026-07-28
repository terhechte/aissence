//! Kimi CLI (kimi-code) session loading.
//!
//! Kimi stores sessions under `$KIMI_CODE_HOME/sessions/<workDirKey>/<sessionId>/`
//! (default `~/.kimi-code/sessions/…`), grouped by working directory. Each
//! session directory holds a `state.json` — whose `workDir` records the absolute
//! project path we scope on — and one event-sourced `agents/<id>/wire.jsonl` per
//! agent.
//!
//! We load the **main** agent's `wire.jsonl`: it carries the user prompts, the
//! assistant's replies and reasoning, and every code-editing tool call. Subagent
//! logs are intentionally skipped — a subagent's result is already inlined into
//! the main log as the corresponding tool result, so loading them would only
//! duplicate content and add exploration noise.
//!
//! `wire.jsonl` is an append-only event stream. The events we care about:
//!
//! - `turn.prompt` — a user prompt (`input` text blocks). Starts a new turn.
//! - `context.append_loop_event` — the assistant's streamed work, one *step* at a
//!   time: `content.part` (`text` / `think`), `tool.call`, and `tool.result`.
//! - `llm.request` — the provider/model for a step.
//! - `usage.record` — per-request token usage, summed into the turn.
//!
//! (`context.append_message` is ignored: it is the derived model context and
//! mixes real prompts with injected system-reminders and tool results.)

use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

pub(super) struct LoadedSessions {
    pub sessions: Vec<ncs::Session>,
    pub warnings: Vec<String>,
}

/// Load every Kimi CLI session whose project directory matches `folder`.
pub(super) fn load_sessions(folder: &Path) -> LoadedSessions {
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let mut sessions = Vec::new();
    let mut warnings = Vec::new();

    for root in sessions_roots() {
        load_from_root(&root, &folder, &mut sessions, &mut warnings);
    }

    sessions.sort_by(|a, b| {
        a.metadata
            .timestamp
            .cmp(&b.metadata.timestamp)
            .then_with(|| a.metadata.session_id.cmp(&b.metadata.session_id))
    });
    LoadedSessions { sessions, warnings }
}

/// Candidate `…/sessions` roots to scan, honoring `KIMI_CODE_HOME`.
fn sessions_roots() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(home) = std::env::var_os("KIMI_CODE_HOME") {
        homes.push(PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("HOME") {
        homes.push(PathBuf::from(home).join(".kimi-code"));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        homes.push(PathBuf::from(profile).join(".kimi-code"));
    }
    let mut roots: Vec<PathBuf> = homes.into_iter().map(|home| home.join("sessions")).collect();
    roots.dedup();
    roots
}

/// Scan one `…/sessions` root (`<workDirKey>/<sessionId>/` two levels deep),
/// loading each session whose `state.json` `workDir` matches `folder`.
fn load_from_root(
    root: &Path,
    folder: &Path,
    sessions: &mut Vec<ncs::Session>,
    warnings: &mut Vec<String>,
) {
    for session_dir in session_dirs(root) {
        let state_path = session_dir.join("state.json");
        let state = match read_json(&state_path) {
            Ok(state) => state,
            // A session directory without a readable state.json isn't ours to
            // report on — skip it silently rather than warn about every one.
            Err(_) => continue,
        };
        let Some(work_dir) = string_at(&state, &["workDir"]) else {
            continue;
        };
        if !directory_matches(Path::new(work_dir), folder) {
            continue;
        }

        let wire = session_dir.join("agents").join("main").join("wire.jsonl");
        if !wire.is_file() {
            continue;
        }
        match parse_wire(&wire, &state, &session_dir) {
            Ok(Some(session)) => sessions.push(session),
            Ok(None) => {}
            Err(error) => warnings.push(error),
        }
    }
}

/// The `<workDirKey>/<sessionId>` directories directly under a sessions root.
fn session_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(work_dirs) = std::fs::read_dir(root) else {
        return dirs;
    };
    for work_dir in work_dirs.flatten() {
        let work_dir = work_dir.path();
        if !work_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&work_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
    }
    dirs.sort();
    dirs
}

/// Turn one `main/wire.jsonl` event stream into a normalized [`ncs::Session`].
///
/// Returns `Ok(None)` when the stream produced no conversation turns.
fn parse_wire(
    path: &Path,
    state: &Value,
    session_dir: &Path,
) -> Result<Option<ncs::Session>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;

    let mut session = ncs::Session::new(path.to_path_buf(), "kimi");
    session.metadata.session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);
    session.metadata.project = string_at(state, &["workDir"]).map(str::to_string);
    session.metadata.timestamp = string_at(state, &["createdAt"]).map(str::to_string);

    // Accumulators for the turn/step we are currently building.
    let mut current = ncs::Turn::default();
    let mut step_blocks: Vec<ncs::ContentBlock> = Vec::new();
    let mut step_time: Option<String> = None;
    let mut last_time: Option<String> = None;

    // Flush the pending assistant step (its text/thinking/tool-call blocks) into
    // the current turn as one assistant message.
    fn flush_step(
        current: &mut ncs::Turn,
        blocks: &mut Vec<ncs::ContentBlock>,
        step_time: &mut Option<String>,
        fallback_time: &Option<String>,
    ) {
        if blocks.is_empty() {
            return;
        }
        current.messages.push(ncs::Message {
            role: ncs::Role::Assistant,
            content: std::mem::take(blocks),
            timestamp: step_time.take().or_else(|| fallback_time.clone()),
        });
    }

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            // Tolerate a truncated trailing line from an in-progress session.
            Err(_) => continue,
        };

        let time = integer_at(&event, &["time"]).map(|value| value.to_string());
        if time.is_some() {
            last_time = time.clone();
        }

        match string_at(&event, &["type"]).unwrap_or("") {
            "turn.prompt" => {
                flush_step(&mut current, &mut step_blocks, &mut step_time, &last_time);
                // A prompt begins a new turn.
                if !current.messages.is_empty() {
                    session.turns.push(std::mem::take(&mut current));
                }
                if let Some(text) = input_text(&event) {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::User,
                        content: vec![ncs::ContentBlock::Text { text }],
                        timestamp: time.clone(),
                    });
                }
            }
            // Steering injects notifications/system-reminders mid-turn; keep them
            // as system messages so they render but don't count as user intent.
            "turn.steer" => {
                flush_step(&mut current, &mut step_blocks, &mut step_time, &last_time);
                if let Some(text) = input_text(&event) {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::System,
                        content: vec![ncs::ContentBlock::Text { text }],
                        timestamp: time.clone(),
                    });
                }
            }
            "context.append_loop_event" => {
                let Some(inner) = event.get("event") else {
                    continue;
                };
                match string_at(inner, &["type"]).unwrap_or("") {
                    "step.begin" => {
                        flush_step(&mut current, &mut step_blocks, &mut step_time, &last_time);
                        step_time = time.clone().or_else(|| last_time.clone());
                    }
                    "content.part" => {
                        let part = inner.get("part").unwrap_or(&Value::Null);
                        match string_at(part, &["type"]).unwrap_or("") {
                            "text" => {
                                if let Some(text) = nonempty_string_at(part, &["text"]) {
                                    step_blocks.push(ncs::ContentBlock::Text {
                                        text: text.to_string(),
                                    });
                                }
                            }
                            "think" => {
                                if let Some(text) = nonempty_string_at(part, &["think"]) {
                                    step_blocks.push(ncs::ContentBlock::Thinking {
                                        text: text.to_string(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    "tool.call" => {
                        let id = string_at(inner, &["toolCallId"])
                            .or_else(|| string_at(inner, &["uuid"]))
                            .unwrap_or("")
                            .to_string();
                        let name = string_at(inner, &["name"]).unwrap_or("unknown").to_string();
                        let input = inner.get("args").cloned().unwrap_or(Value::Null);
                        step_blocks.push(ncs::ContentBlock::ToolUse { id, name, input });
                    }
                    "tool.result" => {
                        // The assistant's tool call(s) precede their results, so
                        // emit the assistant message before the tool output.
                        flush_step(&mut current, &mut step_blocks, &mut step_time, &last_time);
                        let tool_use_id = string_at(inner, &["toolCallId"])
                            .or_else(|| string_at(inner, &["parentUuid"]))
                            .unwrap_or("")
                            .to_string();
                        let content = result_output(inner);
                        let is_error = bool_at(inner, &["result", "isError"]).unwrap_or(false);
                        current.messages.push(ncs::Message {
                            role: ncs::Role::Tool,
                            content: vec![ncs::ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            }],
                            timestamp: time.clone().or_else(|| last_time.clone()),
                        });
                    }
                    _ => {}
                }
            }
            "llm.request" => {
                if session.metadata.provider.is_none() {
                    session.metadata.provider = string_at(&event, &["provider"]).map(str::to_string);
                }
                if session.metadata.model.is_none() {
                    session.metadata.model = string_at(&event, &["modelAlias"])
                        .or_else(|| string_at(&event, &["model"]))
                        .map(str::to_string);
                }
            }
            "usage.record" => {
                if let Some(usage) = usage_record(&event) {
                    let total = current.token_usage.get_or_insert_with(Default::default);
                    total.input += usage.input;
                    total.output += usage.output;
                    add_optional(&mut total.cache_read, usage.cache_read);
                    add_optional(&mut total.cache_create, usage.cache_create);
                }
            }
            _ => {}
        }
    }

    flush_step(&mut current, &mut step_blocks, &mut step_time, &last_time);
    if !current.messages.is_empty() {
        session.turns.push(current);
    }

    if session.turns.is_empty() {
        return Ok(None);
    }
    Ok(Some(session))
}

/// Concatenate the `text` blocks of an event's `input` array.
fn input_text(event: &Value) -> Option<String> {
    let blocks = event.get("input")?.as_array()?;
    let mut out = String::new();
    for block in blocks {
        if string_at(block, &["type"]) == Some("text")
            && let Some(text) = nonempty_string_at(block, &["text"])
        {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Extract a tool result's textual output (`result.output`, else the raw result).
fn result_output(inner: &Value) -> String {
    if let Some(output) = string_at(inner, &["result", "output"]) {
        return output.to_string();
    }
    match inner.get("result") {
        Some(Value::String(text)) => text.clone(),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

/// Map a `usage.record` event to an [`ncs::TokenUsage`].
///
/// Kimi separates cached input (`inputCacheRead` / `inputCacheCreation`) from
/// fresh input (`inputOther`), matching the crate's input/cache split.
fn usage_record(event: &Value) -> Option<ncs::TokenUsage> {
    let usage = event.get("usage")?;
    Some(ncs::TokenUsage {
        input: integer_at(usage, &["inputOther"]).unwrap_or(0).max(0) as u64,
        output: integer_at(usage, &["output"]).unwrap_or(0).max(0) as u64,
        cache_read: integer_at(usage, &["inputCacheRead"]).map(|value| value.max(0) as u64),
        cache_create: integer_at(usage, &["inputCacheCreation"]).map(|value| value.max(0) as u64),
        model: string_at(event, &["model"]).map(str::to_string),
    })
}

fn add_optional(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target.get_or_insert(0) += value;
    }
}

fn directory_matches(directory: &Path, folder: &Path) -> bool {
    let directory = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    directory == folder || directory.starts_with(folder)
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(key))
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    value_at(value, path)?.as_str()
}

fn nonempty_string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    string_at(value, path).filter(|text| !text.trim().is_empty())
}

fn integer_at(value: &Value, path: &[&str]) -> Option<i64> {
    let value = value_at(value, path)?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.parse().ok())
}

fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    value_at(value, path)?.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("aissence-kimi-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Write a `<root>/sessions/<key>/<id>/` session with a `state.json` and a
    /// `main/wire.jsonl` built from the given event lines.
    fn write_session(root: &Path, key: &str, id: &str, work_dir: &Path, wire: &[&str]) {
        let dir = root.join("sessions").join(key).join(id);
        let main = dir.join("agents").join("main");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::write(
            dir.join("state.json"),
            format!(
                r#"{{"createdAt":"2026-07-20T15:11:56.892Z","workDir":{}}}"#,
                serde_json::to_string(&work_dir.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        std::fs::write(main.join("wire.jsonl"), wire.join("\n")).unwrap();
    }

    #[test]
    fn parses_prompts_steps_tools_and_usage() {
        let root = temp_dir("parse");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();

        let wire = [
            r#"{"type":"metadata","protocol_version":"1.4","created_at":1784560316903}"#,
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"Rename the widget"}],"origin":{"kind":"user"},"time":1784560372445}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"step.begin","step":1,"turnId":"0"},"time":1784560372500}"#,
            r#"{"type":"llm.request","provider":"kimi","model":"k3","modelAlias":"kimi-code/k3","turnStep":"0.1","time":1784560373000}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","step":1,"part":{"type":"think","think":"Plan the edit"}},"time":1784560373100}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","step":1,"part":{"type":"text","text":"I'll rename it."}},"time":1784560373200}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","step":1,"toolCallId":"call_1","name":"Edit","args":{"file_path":"src/lib.rs","old_string":"widget","new_string":"gadget"}},"time":1784560373300}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"tool.result","toolCallId":"call_1","result":{"output":"Edited"}},"time":1784560373400}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"step.end","step":1,"turnId":"0"},"time":1784560373500}"#,
            r#"{"type":"usage.record","model":"kimi-code/k3","usage":{"inputOther":30,"output":12,"inputCacheRead":4,"inputCacheCreation":1},"usageScope":"turn","time":1784560373600}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","step":2,"part":{"type":"text","text":"Done."}},"time":1784560373700}"#,
            r#"{"type":"usage.record","model":"kimi-code/k3","usage":{"inputOther":10,"output":3,"inputCacheRead":6,"inputCacheCreation":0},"usageScope":"turn","time":1784560373800}"#,
        ];
        write_session(&root, "wd_project_abc", "ses_1", &project, &wire);

        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        load_from_root(&root.join("sessions"), &project, &mut sessions, &mut warnings);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.format, "kimi");
        assert_eq!(session.metadata.provider.as_deref(), Some("kimi"));
        assert_eq!(session.metadata.model.as_deref(), Some("kimi-code/k3"));
        assert_eq!(session.metadata.session_id.as_deref(), Some("ses_1"));

        // user prompt, assistant (think+text+tool), tool result, trailing assistant.
        assert_eq!(session.messages_by_role(ncs::Role::User), 1);
        assert_eq!(session.messages_by_role(ncs::Role::Assistant), 2);
        assert_eq!(session.messages_by_role(ncs::Role::Tool), 1);

        // The edit tool call is preserved for code-change extraction.
        let (tool, input) = session.tool_uses().next().unwrap();
        assert_eq!(tool, "Edit");
        assert_eq!(input["file_path"], "src/lib.rs");

        // Token usage is summed across both usage records of the turn.
        let total = session.total_tokens();
        assert_eq!(total.input, 40);
        assert_eq!(total.output, 15);
        assert_eq!(total.cache_read, Some(10));
        assert_eq!(total.cache_create, Some(1));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sessions_for_other_projects_are_skipped() {
        let root = temp_dir("scope");
        let project = root.join("project");
        let other = root.join("other");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        let wire =
            [r#"{"type":"turn.prompt","input":[{"type":"text","text":"hi"}],"time":1784560372445}"#];
        write_session(&root, "wd_other", "ses_other", &other, &wire);

        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        load_from_root(&root.join("sessions"), &project, &mut sessions, &mut warnings);

        assert!(sessions.is_empty());
        assert!(warnings.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
