//! Google Antigravity `brain` transcript parser.
//!
//! The plaintext hook transcript is a step log rather than a message log.
//! Records can be appended out of order, and tool results have no IDs, so this
//! parser sorts by `step_index` and pairs calls/results positionally.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

use super::common::{
    directory_matches, integer_at, nonempty_string_at, string_at, text_or_json, value_at,
};

pub(super) struct LoadedSessions {
    pub sessions: Vec<ncs::Session>,
    pub warnings: Vec<String>,
}

pub(super) fn load_sessions(folder: &Path) -> LoadedSessions {
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let mut files = Vec::new();
    for root in roots() {
        let projects = history_projects(&root.join("history.jsonl"));
        let mut root_files = Vec::new();
        collect_transcripts(&root.join("brain"), &mut root_files);
        files.extend(root_files.into_iter().map(|path| {
            let project = conversation_id(&path)
                .and_then(|id| projects.get(&id))
                .map(PathBuf::from);
            (path, project)
        }));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    for (path, project) in files {
        match parse_file(&path, &folder, project.as_deref()) {
            Ok(Some(session)) => sessions.push(session),
            Ok(None) => {}
            Err(error) => warnings.push(error),
        }
    }
    LoadedSessions { sessions, warnings }
}

fn history_projects(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|entry| {
            Some((
                nonempty_string_at(&entry, &["conversationId"])?.to_string(),
                nonempty_string_at(&entry, &["workspace"])?.to_string(),
            ))
        })
        .collect()
}

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = std::env::var_os("ANTIGRAVITY_HOME") {
        roots.push(PathBuf::from(root));
    }
    for home in [
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(home.join(".gemini/antigravity"));
        roots.push(home.join(".gemini/antigravity-cli"));
        roots.push(home.join(".gemini/antigravity-ide"));
    }
    roots.sort();
    roots.dedup();
    roots
}

fn collect_transcripts(brain: &Path, output: &mut Vec<PathBuf>) {
    let Ok(conversations) = std::fs::read_dir(brain) else {
        return;
    };
    for conversation in conversations.flatten() {
        if !conversation.path().is_dir() {
            continue;
        }
        let logs = conversation.path().join(".system_generated/logs");
        let full = logs.join("transcript_full.jsonl");
        let short = logs.join("transcript.jsonl");
        if full.is_file() {
            output.push(full);
        } else if short.is_file() {
            output.push(short);
        }
    }
}

fn parse_file(
    path: &Path,
    folder: &Path,
    project_hint: Option<&Path>,
) -> Result<Option<ncs::Session>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut records: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|value| integer_at(value, &["step_index"]).is_some())
        .collect();
    records.sort_by_key(|value| integer_at(value, &["step_index"]).unwrap_or(i64::MAX));
    let project_matches = project_hint
        .map(|project| directory_matches(project, folder))
        .unwrap_or_else(|| references_project(&records, folder));
    if records.is_empty() || !project_matches {
        return Ok(None);
    }

    let mut session = ncs::Session::new(path.to_path_buf(), "antigravity");
    session.metadata.session_id = conversation_id(path);
    session.metadata.timestamp = records.first().and_then(record_timestamp);
    session.metadata.project = Some(
        project_hint
            .unwrap_or(folder)
            .to_string_lossy()
            .into_owned(),
    );
    session.metadata.provider = Some("google".to_string());

    let mut current = ncs::Turn::default();
    let mut pending: VecDeque<String> = VecDeque::new();
    for record in &records {
        let kind = string_at(record, &["type"]).unwrap_or("");
        let timestamp = record_timestamp(record);
        match kind {
            "USER_INPUT" => {
                if !current.messages.is_empty() {
                    session.turns.push(std::mem::take(&mut current));
                }
                let raw = string_at(record, &["content"]).unwrap_or("");
                let (request, remainder) = split_user_input(raw);
                if !request.is_empty() {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::User,
                        content: vec![ncs::ContentBlock::Text { text: request }],
                        timestamp: timestamp.clone(),
                    });
                }
                if session.metadata.model.is_none() {
                    session.metadata.model = model_from_context(&remainder);
                }
                if !remainder.is_empty() {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::System,
                        content: vec![ncs::ContentBlock::Text { text: remainder }],
                        timestamp,
                    });
                }
            }
            "PLANNER_RESPONSE" => {
                pending.clear();
                let mut content = Vec::new();
                if let Some(thinking) = nonempty_string_at(record, &["thinking"]) {
                    content.push(ncs::ContentBlock::Thinking {
                        text: thinking.to_string(),
                    });
                }
                if let Some(text) = nonempty_string_at(record, &["content"]) {
                    content.push(ncs::ContentBlock::Text {
                        text: text.to_string(),
                    });
                }
                let step = integer_at(record, &["step_index"]).unwrap_or(0);
                for (index, call) in record
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let id = format!("step-{step}:call-{index}");
                    pending.push_back(id.clone());
                    content.push(ncs::ContentBlock::ToolUse {
                        id,
                        name: string_at(call, &["name"]).unwrap_or("unknown").to_string(),
                        input: value_at(call, &["args"]).cloned().unwrap_or(Value::Null),
                    });
                }
                if !content.is_empty() {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::Assistant,
                        content,
                        timestamp,
                    });
                }
            }
            kind if is_result(kind)
                || (matches!(kind, "GENERIC" | "ASK_QUESTION") && !pending.is_empty()) =>
            {
                let step = integer_at(record, &["step_index"]).unwrap_or(0);
                let tool_use_id = pending
                    .pop_front()
                    .unwrap_or_else(|| format!("step-{step}:orphan"));
                let content = nonempty_string_at(record, &["content"])
                    .or_else(|| nonempty_string_at(record, &["error"]))
                    .unwrap_or("")
                    .to_string();
                current.messages.push(ncs::Message {
                    role: ncs::Role::Tool,
                    content: vec![ncs::ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error: nonempty_string_at(record, &["error"]).is_some(),
                    }],
                    timestamp,
                });
            }
            "CONVERSATION_HISTORY"
            | "SYSTEM_MESSAGE"
            | "CHECKPOINT"
            | "ERROR_MESSAGE"
            | "GENERIC"
            | "ASK_QUESTION" => {
                let content = nonempty_string_at(record, &["error"])
                    .or_else(|| nonempty_string_at(record, &["content"]));
                if let Some(content) = content {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::System,
                        content: vec![ncs::ContentBlock::Text {
                            text: content.to_string(),
                        }],
                        timestamp,
                    });
                }
            }
            _ => {}
        }
    }
    if !current.messages.is_empty() {
        session.turns.push(current);
    }
    Ok((!session.turns.is_empty()).then_some(session))
}

fn conversation_id(path: &Path) -> Option<String> {
    path.ancestors()
        .find(|ancestor| ancestor.join(".system_generated/logs").is_dir())
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn references_project(records: &[Value], folder: &Path) -> bool {
    let folder_text = folder.to_string_lossy();
    records.iter().any(|record| {
        record
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|call| {
                let args = call.get("args").unwrap_or(&Value::Null);
                if let Some(cwd) = nonempty_string_at(args, &["Cwd"])
                    .or_else(|| nonempty_string_at(args, &["cwd"]))
                {
                    return directory_matches(Path::new(cwd), folder);
                }
                args.to_string().contains(folder_text.as_ref())
            })
            || text_or_json(record.get("content")).contains(folder_text.as_ref())
    })
}

fn is_result(kind: &str) -> bool {
    matches!(
        kind,
        "RUN_COMMAND"
            | "VIEW_FILE"
            | "LIST_DIRECTORY"
            | "GREP_SEARCH"
            | "SEARCH_WEB"
            | "READ_URL_CONTENT"
            | "CODE_ACTION"
    )
}

fn split_user_input(content: &str) -> (String, String) {
    const OPEN: &str = "<USER_REQUEST>";
    const CLOSE: &str = "</USER_REQUEST>";
    let Some(start) = content.find(OPEN) else {
        return (content.trim().to_string(), String::new());
    };
    let Some(end) = content[start + OPEN.len()..]
        .find(CLOSE)
        .map(|offset| offset + start + OPEN.len())
    else {
        return (content.trim().to_string(), String::new());
    };
    let request = content[start + OPEN.len()..end].trim().to_string();
    let remainder = format!("{}{}", &content[..start], &content[end + CLOSE.len()..])
        .trim()
        .to_string();
    (request, remainder)
}

fn model_from_context(context: &str) -> Option<String> {
    let marker = "`Model Selection`";
    let after = context
        .find(marker)
        .map(|index| &context[index + marker.len()..])?;
    let after = after.find(" to ").map(|index| &after[index + 4..])?;
    Some(after.split(". ").next().unwrap_or(after).trim().to_string())
}

fn record_timestamp(record: &Value) -> Option<String> {
    nonempty_string_at(record, &["created_at"]).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_steps_and_pairs_tool_results() {
        let directory =
            std::env::temp_dir().join(format!("aissence-antigravity-{}", std::process::id()));
        let project = directory.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let logs = directory.join("conv/.system_generated/logs");
        std::fs::create_dir_all(&logs).unwrap();
        let path = logs.join("transcript_full.jsonl");
        let cwd = serde_json::to_string(&project.to_string_lossy()).unwrap();
        std::fs::write(
            &path,
            [
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","created_at":"2026-01-01T00:00:00Z","content":"<USER_REQUEST>Run it</USER_REQUEST>\nThe user changed setting `Model Selection` from None to Gemini 3.5 Flash (Medium). No need."}"#.to_string(),
                r#"{"step_index":2,"source":"MODEL","type":"RUN_COMMAND","created_at":"2026-01-01T00:00:02Z","content":"ok"}"#.to_string(),
                format!(r#"{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-01-01T00:00:01Z","tool_calls":[{{"name":"run_command","args":{{"Cwd":{cwd},"CommandLine":"test"}}}}]}}"#),
            ]
            .join("\n"),
        )
        .unwrap();

        let session = parse_file(&path, &project, None).unwrap().unwrap();
        assert_eq!(session.metadata.session_id.as_deref(), Some("conv"));
        assert_eq!(
            session.metadata.model.as_deref(),
            Some("Gemini 3.5 Flash (Medium)")
        );
        assert_eq!(session.tool_uses().next().unwrap().0, "run_command");
        assert_eq!(session.tool_results().next(), Some(("ok", false)));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn history_map_attributes_path_free_conversations() {
        let directory =
            std::env::temp_dir().join(format!("aissence-antigravity-history-{}", std::process::id()));
        let project = directory.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let path = directory.join("history.jsonl");
        std::fs::write(
            &path,
            format!(
                r#"{{"conversationId":"conv","workspace":{}}}"#,
                serde_json::to_string(&project.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        let projects = history_projects(&path);
        assert_eq!(projects.get("conv").map(String::as_str), project.to_str());
        let _ = std::fs::remove_dir_all(directory);
    }
}
