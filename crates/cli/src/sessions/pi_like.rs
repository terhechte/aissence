//! Session parser shared by Pi and Oh My Pi.
//!
//! Both agents use Pi's version-3 JSONL format: a session header followed by
//! tree-shaped entries linked through `id`/`parentId`. We reconstruct the
//! active branch ending at the final entry, which avoids showing messages from
//! branches the user abandoned.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

use super::common::{
    add_usage, bool_at, collect_files, directory_matches, integer_at, nonempty_string_at,
    string_at, text_or_json, value_at,
};

pub(super) struct LoadedSessions {
    pub sessions: Vec<ncs::Session>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
pub(super) enum Flavor {
    Pi,
    Omp,
}

impl Flavor {
    fn format(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Omp => "omp",
        }
    }

    fn roots(self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        match self {
            Self::Pi => {
                if let Some(root) = std::env::var_os("PI_CODING_AGENT_DIR") {
                    roots.push(PathBuf::from(root).join("sessions"));
                }
                if let Some(home) = std::env::var_os("HOME") {
                    roots.push(PathBuf::from(home).join(".pi/agent/sessions"));
                }
                if let Some(profile) = std::env::var_os("USERPROFILE") {
                    roots.push(PathBuf::from(profile).join(".pi/agent/sessions"));
                }
            }
            Self::Omp => {
                if let Some(root) = std::env::var_os("OMP_HOME") {
                    let root = PathBuf::from(root);
                    roots.push(if root.ends_with("sessions") {
                        root
                    } else {
                        root.join("agent/sessions")
                    });
                }
                if let Some(home) = std::env::var_os("HOME") {
                    roots.push(PathBuf::from(home).join(".omp/agent/sessions"));
                }
                if let Some(profile) = std::env::var_os("USERPROFILE") {
                    roots.push(PathBuf::from(profile).join(".omp/agent/sessions"));
                }
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }
}

pub(super) fn load_sessions(folder: &Path, flavor: Flavor) -> LoadedSessions {
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let mut files = Vec::new();
    for root in flavor.roots() {
        collect_files(&root, &["jsonl"], &mut files);
    }
    files.sort();
    files.dedup();

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    for path in files {
        if !session_project(&path)
            .is_some_and(|project| directory_matches(Path::new(&project), &folder))
        {
            continue;
        }
        match parse_file(&path, flavor) {
            Ok(Some(session)) => sessions.push(session),
            Ok(_) => {}
            Err(error) => warnings.push(error),
        }
    }
    LoadedSessions { sessions, warnings }
}

fn session_project(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    std::io::BufReader::new(file)
        .lines()
        .take(3)
        .filter_map(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .find(|value| string_at(value, &["type"]) == Some("session"))
        .and_then(|value| string_at(&value, &["cwd"]).map(str::to_string))
}

fn parse_file(path: &Path, flavor: Flavor) -> Result<Option<ncs::Session>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut values = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        // In-progress sessions can end in a partial JSON line.
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            values.push(value);
        }
    }
    let Some((header_index, header)) = values
        .iter()
        .enumerate()
        .find(|(_, value)| string_at(value, &["type"]) == Some("session"))
    else {
        return Ok(None);
    };

    let mut session = ncs::Session::new(path.to_path_buf(), flavor.format());
    session.metadata.session_id = string_at(header, &["id"]).map(str::to_string);
    session.metadata.timestamp = timestamp(header, &["timestamp"]);
    session.metadata.project = string_at(header, &["cwd"]).map(str::to_string);

    let entries = active_branch(&values[header_index + 1..]);
    let mut current = ncs::Turn::default();
    for entry in entries {
        match string_at(entry, &["type"]).unwrap_or("") {
            "message" => normalize_message(entry, &mut session, &mut current),
            "compaction" | "branch_summary" => {
                if let Some(summary) = nonempty_string_at(entry, &["summary"]) {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::System,
                        content: vec![ncs::ContentBlock::Text {
                            text: summary.to_string(),
                        }],
                        timestamp: timestamp(entry, &["timestamp"]),
                    });
                }
                if let Some(usage) = entry.get("usage").and_then(token_usage) {
                    add_usage(&mut current.token_usage, usage);
                }
            }
            "custom_message" => {
                let blocks = content_blocks(entry.get("content"));
                if !blocks.is_empty() {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::System,
                        content: blocks,
                        timestamp: timestamp(entry, &["timestamp"]),
                    });
                }
            }
            "model_change" => {
                if session.metadata.provider.is_none() {
                    session.metadata.provider = string_at(entry, &["provider"]).map(str::to_string);
                }
                if session.metadata.model.is_none() {
                    session.metadata.model = string_at(entry, &["modelId"])
                        .or_else(|| string_at(entry, &["model"]))
                        .map(str::to_string);
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

fn active_branch(entries: &[Value]) -> Vec<&Value> {
    let indexed: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, value)| string_at(value, &["id"]).map(|id| (id, index)))
        .collect();
    if indexed.is_empty() {
        return entries.iter().collect();
    }

    let by_id: HashMap<_, _> = indexed.iter().map(|(id, index)| (*id, *index)).collect();
    let mut branch = Vec::new();
    let mut cursor = indexed.last().map(|(_, index)| *index);
    while let Some(index) = cursor {
        let entry = &entries[index];
        branch.push(entry);
        cursor = string_at(entry, &["parentId"]).and_then(|id| by_id.get(id).copied());
    }
    branch.reverse();
    branch
}

fn normalize_message(entry: &Value, session: &mut ncs::Session, current: &mut ncs::Turn) {
    let message = entry.get("message").unwrap_or(entry);
    let Some(role) = string_at(message, &["role"]) else {
        return;
    };
    let timestamp = timestamp(message, &["timestamp"]).or_else(|| timestamp(entry, &["timestamp"]));

    if role == "user" && !current.messages.is_empty() {
        session.turns.push(std::mem::take(current));
    }

    match role {
        "user" | "assistant" | "system" => {
            let role = match role {
                "user" => ncs::Role::User,
                "assistant" => ncs::Role::Assistant,
                _ => ncs::Role::System,
            };
            let blocks = message_blocks(message);
            if !blocks.is_empty() {
                current.messages.push(ncs::Message {
                    role,
                    content: blocks,
                    timestamp,
                });
            }
            if role == ncs::Role::Assistant {
                if session.metadata.provider.is_none() {
                    session.metadata.provider =
                        string_at(message, &["provider"]).map(str::to_string);
                }
                if session.metadata.model.is_none() {
                    session.metadata.model = string_at(message, &["model"])
                        .or_else(|| string_at(message, &["modelId"]))
                        .map(str::to_string);
                }
                if let Some(usage) = message.get("usage").and_then(token_usage) {
                    add_usage(&mut current.token_usage, usage);
                }
            }
        }
        "toolResult" | "tool" => {
            let tool_use_id = string_at(message, &["toolCallId"])
                .or_else(|| string_at(message, &["tool_use_id"]))
                .unwrap_or("")
                .to_string();
            let content = match message.get("content") {
                Some(Value::Array(blocks)) => blocks
                    .iter()
                    .filter_map(|block| nonempty_string_at(block, &["text"]))
                    .collect::<Vec<_>>()
                    .join("\n"),
                value => text_or_json(value),
            };
            current.messages.push(ncs::Message {
                role: ncs::Role::Tool,
                content: vec![ncs::ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error: bool_at(message, &["isError"]).unwrap_or(false),
                }],
                timestamp,
            });
        }
        "bashExecution" => {
            let id = format!("bash-{}", string_at(entry, &["id"]).unwrap_or("execution"));
            let command = string_at(message, &["command"]).unwrap_or("");
            current.messages.push(ncs::Message {
                role: ncs::Role::Assistant,
                content: vec![ncs::ContentBlock::ToolUse {
                    id: id.clone(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": command}),
                }],
                timestamp: timestamp.clone(),
            });
            current.messages.push(ncs::Message {
                role: ncs::Role::Tool,
                content: vec![ncs::ContentBlock::ToolResult {
                    tool_use_id: id,
                    content: string_at(message, &["output"]).unwrap_or("").to_string(),
                    is_error: integer_at(message, &["exitCode"]).is_some_and(|code| code != 0),
                }],
                timestamp,
            });
        }
        "custom" => {
            let blocks = content_blocks(message.get("content"));
            if !blocks.is_empty() {
                current.messages.push(ncs::Message {
                    role: ncs::Role::System,
                    content: blocks,
                    timestamp,
                });
            }
        }
        "branchSummary" | "compactionSummary" => {
            if let Some(summary) = nonempty_string_at(message, &["summary"]) {
                current.messages.push(ncs::Message {
                    role: ncs::Role::System,
                    content: vec![ncs::ContentBlock::Text {
                        text: summary.to_string(),
                    }],
                    timestamp,
                });
            }
        }
        _ => {}
    }
}

fn message_blocks(message: &Value) -> Vec<ncs::ContentBlock> {
    content_blocks(message.get("content"))
}

fn content_blocks(content: Option<&Value>) -> Vec<ncs::ContentBlock> {
    let Some(content) = content else {
        return Vec::new();
    };
    if let Some(text) = content.as_str() {
        return (!text.trim().is_empty())
            .then(|| ncs::ContentBlock::Text {
                text: text.to_string(),
            })
            .into_iter()
            .collect();
    }
    let Some(parts) = content.as_array() else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match string_at(part, &["type"]).unwrap_or("") {
            "text" => nonempty_string_at(part, &["text"]).map(|text| ncs::ContentBlock::Text {
                text: text.to_string(),
            }),
            "thinking" => nonempty_string_at(part, &["thinking"])
                .or_else(|| nonempty_string_at(part, &["text"]))
                .map(|text| ncs::ContentBlock::Thinking {
                    text: text.to_string(),
                }),
            "toolCall" | "tool_use" => Some(ncs::ContentBlock::ToolUse {
                id: string_at(part, &["id"]).unwrap_or("").to_string(),
                name: string_at(part, &["name"]).unwrap_or("unknown").to_string(),
                input: value_at(part, &["arguments"])
                    .or_else(|| value_at(part, &["input"]))
                    .cloned()
                    .unwrap_or(Value::Null),
            }),
            _ => None,
        })
        .collect()
}

fn token_usage(usage: &Value) -> Option<ncs::TokenUsage> {
    Some(ncs::TokenUsage {
        input: integer_at(usage, &["input"]).unwrap_or(0).max(0) as u64,
        output: integer_at(usage, &["output"]).unwrap_or(0).max(0) as u64,
        cache_read: integer_at(usage, &["cacheRead"]).map(|value| value.max(0) as u64),
        cache_create: integer_at(usage, &["cacheWrite"]).map(|value| value.max(0) as u64),
        model: None,
    })
}

fn timestamp(value: &Value, path: &[&str]) -> Option<String> {
    nonempty_string_at(value, path)
        .map(str::to_string)
        .or_else(|| integer_at(value, path).map(|value| value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_branch_tools_and_usage() {
        let directory = std::env::temp_dir().join(format!("aissence-pi-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let file = directory.join("session.jsonl");
        std::fs::write(
            &file,
            [
                r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-01-01T00:00:00Z","cwd":"/project"}"#,
                r#"{"type":"message","id":"a","parentId":null,"timestamp":"2026-01-01T00:00:01Z","message":{"role":"user","content":"Do it","timestamp":1767225601000}}"#,
                r#"{"type":"message","id":"abandoned","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"old branch"}]}}"#,
                r#"{"type":"message","id":"b","parentId":"a","message":{"role":"assistant","provider":"openai","model":"gpt-5","usage":{"input":10,"output":3,"cacheRead":2,"cacheWrite":1},"content":[{"type":"thinking","thinking":"plan"},{"type":"toolCall","id":"c1","name":"write","arguments":{"path":"x"}}]}}"#,
                r#"{"type":"message","id":"c","parentId":"b","message":{"role":"toolResult","toolCallId":"c1","content":[{"type":"text","text":"done"}],"isError":false}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let session = parse_file(&file, Flavor::Pi).unwrap().unwrap();
        assert_eq!(session.message_count(), 3);
        assert_eq!(session.metadata.model.as_deref(), Some("gpt-5"));
        assert_eq!(session.tool_uses().next().unwrap().0, "write");
        assert_eq!(session.tool_results().next(), Some(("done", false)));
        assert_eq!(session.total_tokens().cache_read, Some(2));
        assert!(
            session
                .turns
                .iter()
                .flat_map(|turn| &turn.messages)
                .all(|message| {
                    !message.content.iter().any(
                |block| matches!(block, ncs::ContentBlock::Text { text } if text == "old branch")
            )
                })
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
