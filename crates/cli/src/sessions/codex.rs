//! Parser for current OpenAI Codex rollout JSONL files.
//!
//! The upstream normalizer bundled by the project targets an older payload
//! shape. Current rollouts use `session_meta.payload.id`, event messages for
//! real user prompts, array-valued response content, and `last_token_usage`.
//! Internal approval-guardian rollouts are skipped: their synthetic
//! user/assistant exchange is implementation metadata, not part of the user's
//! coding conversation.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

use super::common::{
    add_usage, collect_files, directory_matches, integer_at, nonempty_string_at, string_at,
    text_or_json, value_at,
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
        match parse_file(&path) {
            Ok(Some(session)) => sessions.push(session),
            Ok(_) => {}
            Err(error) => warnings.push(error),
        }
    }
    LoadedSessions { sessions, warnings }
}

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        roots.push(PathBuf::from(home).join("sessions"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".codex/sessions"));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        roots.push(PathBuf::from(profile).join(".codex/sessions"));
    }
    roots.sort();
    roots.dedup();
    roots
}

pub(super) fn is_codex_session(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    std::io::BufReader::new(file)
        .lines()
        .take(8)
        .filter_map(Result::ok)
        .any(|line| {
            serde_json::from_str::<Value>(&line)
                .ok()
                .is_some_and(|value| string_at(&value, &["type"]) == Some("session_meta"))
        })
}

fn session_project(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    std::io::BufReader::new(file)
        .lines()
        .take(8)
        .filter_map(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .find(|value| string_at(value, &["type"]) == Some("session_meta"))
        .and_then(|value| string_at(&value, &["payload", "cwd"]).map(str::to_string))
}

fn parse_file(path: &Path) -> Result<Option<ncs::Session>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let Some(meta) = entries
        .iter()
        .find(|entry| string_at(entry, &["type"]) == Some("session_meta"))
        .and_then(|entry| entry.get("payload"))
    else {
        return Ok(None);
    };

    if is_approval_guardian(meta) {
        return Ok(None);
    }

    let has_user_events = entries.iter().any(|entry| {
        string_at(entry, &["type"]) == Some("event_msg")
            && string_at(entry, &["payload", "type"]) == Some("user_message")
    });
    let has_assistant_items = entries.iter().any(|entry| {
        string_at(entry, &["type"]) == Some("response_item")
            && string_at(entry, &["payload", "type"]) == Some("message")
            && string_at(entry, &["payload", "role"]) == Some("assistant")
    });

    let mut session = ncs::Session::new(path.to_path_buf(), "codex");
    session.metadata.session_id = string_at(meta, &["id"])
        .or_else(|| string_at(meta, &["session_id"]))
        .map(str::to_string);
    session.metadata.timestamp = timestamp_at(meta, &["timestamp"]);
    session.metadata.project = string_at(meta, &["cwd"]).map(str::to_string);
    session.metadata.provider = string_at(meta, &["model_provider"])
        .map(str::to_string)
        .or_else(|| Some("openai".to_string()));
    session.subagent_type = Some("interactive".to_string());

    let mut current = ncs::Turn::default();
    let mut current_model = session.metadata.model.clone();
    for entry in &entries {
        let outer_type = string_at(entry, &["type"]).unwrap_or("");
        let payload = entry.get("payload").unwrap_or(&Value::Null);
        let payload_type = string_at(payload, &["type"]).unwrap_or("");
        let timestamp = timestamp_at(entry, &["timestamp"]);

        if outer_type == "turn_context" {
            if let Some(model) = string_at(payload, &["model"]) {
                current_model = Some(model.to_string());
                if session.metadata.model.is_none() {
                    session.metadata.model = Some(model.to_string());
                }
            }
            continue;
        }

        if outer_type == "event_msg" {
            match payload_type {
                "user_message" => {
                    let text = nonempty_string_at(payload, &["message"])
                        .or_else(|| nonempty_string_at(payload, &["content"]));
                    if let Some(text) = text {
                        start_user_turn(&mut session, &mut current, text, timestamp);
                    }
                }
                "agent_message" if !has_assistant_items => {
                    if let Some(text) = nonempty_string_at(payload, &["message"])
                        .or_else(|| nonempty_string_at(payload, &["content"]))
                    {
                        current.messages.push(ncs::Message {
                            role: ncs::Role::Assistant,
                            content: vec![ncs::ContentBlock::Text {
                                text: text.to_string(),
                            }],
                            timestamp,
                        });
                    }
                }
                "token_count" => {
                    if let Some(usage) = value_at(payload, &["info", "last_token_usage"])
                        .and_then(|usage| token_usage(usage, current_model.as_deref()))
                    {
                        add_usage(&mut current.token_usage, usage);
                    } else if let Some(usage) = value_at(payload, &["info", "total_token_usage"])
                        .and_then(|usage| token_usage(usage, current_model.as_deref()))
                    {
                        // Older rollouts only expose a cumulative snapshot.
                        // Replace rather than add repeated snapshots.
                        current.token_usage = Some(usage);
                    }
                }
                _ => {}
            }
            continue;
        }

        if outer_type != "response_item" {
            continue;
        }
        match payload_type {
            "message" => {
                let role = string_at(payload, &["role"]).unwrap_or("");
                if role == "user" && has_user_events {
                    continue;
                }
                let blocks = response_message_blocks(payload);
                if blocks.is_empty() {
                    continue;
                }
                if role == "user" {
                    let text = blocks
                        .iter()
                        .filter_map(|block| match block {
                            ncs::ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        start_user_turn(&mut session, &mut current, &text, timestamp);
                    }
                } else if role == "assistant" {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::Assistant,
                        content: blocks,
                        timestamp,
                    });
                }
            }
            "reasoning" => {
                let text = payload
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|part| nonempty_string_at(part, &["text"]))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    current.messages.push(ncs::Message {
                        role: ncs::Role::Assistant,
                        content: vec![ncs::ContentBlock::Thinking { text }],
                        timestamp,
                    });
                }
            }
            "function_call" | "custom_tool_call" => {
                let id = string_at(payload, &["call_id"])
                    .or_else(|| string_at(payload, &["id"]))
                    .unwrap_or("")
                    .to_string();
                let name = string_at(payload, &["name"])
                    .unwrap_or("unknown")
                    .to_string();
                let input_value = value_at(payload, &["arguments"])
                    .or_else(|| value_at(payload, &["input"]))
                    .cloned()
                    .unwrap_or(Value::Null);
                let input = input_value
                    .as_str()
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or(input_value);
                current.messages.push(ncs::Message {
                    role: ncs::Role::Assistant,
                    content: vec![ncs::ContentBlock::ToolUse { id, name, input }],
                    timestamp,
                });
            }
            "function_call_output" | "custom_tool_call_output" => {
                let tool_use_id = string_at(payload, &["call_id"])
                    .or_else(|| string_at(payload, &["id"]))
                    .unwrap_or("")
                    .to_string();
                let content = text_or_json(
                    value_at(payload, &["output"]).or_else(|| value_at(payload, &["content"])),
                );
                let lower = content.to_ascii_lowercase();
                let is_error = lower.starts_with("error:")
                    || lower.contains("exit code: 1")
                    || lower.contains("\"exit_code\":1");
                current.messages.push(ncs::Message {
                    role: ncs::Role::Tool,
                    content: vec![ncs::ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    }],
                    timestamp,
                });
            }
            _ => {}
        }
    }
    if !current.messages.is_empty() {
        session.turns.push(current);
    }
    Ok((!session.turns.is_empty()).then_some(session))
}

fn is_approval_guardian(meta: &Value) -> bool {
    string_at(meta, &["thread_source"]) == Some("subagent")
        && string_at(meta, &["source", "subagent", "other"]) == Some("guardian")
}

fn start_user_turn(
    session: &mut ncs::Session,
    current: &mut ncs::Turn,
    text: &str,
    timestamp: Option<String>,
) {
    if !current.messages.is_empty() {
        session.turns.push(std::mem::take(current));
    }
    current.messages.push(ncs::Message {
        role: ncs::Role::User,
        content: vec![ncs::ContentBlock::Text {
            text: text.to_string(),
        }],
        timestamp,
    });
}

fn response_message_blocks(payload: &Value) -> Vec<ncs::ContentBlock> {
    match payload.get("content") {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            vec![ncs::ContentBlock::Text { text: text.clone() }]
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                let kind = string_at(part, &["type"]).unwrap_or("");
                if matches!(kind, "input_text" | "output_text" | "text") {
                    nonempty_string_at(part, &["text"]).map(|text| ncs::ContentBlock::Text {
                        text: text.to_string(),
                    })
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn token_usage(value: &Value, model: Option<&str>) -> Option<ncs::TokenUsage> {
    let input = integer_at(value, &["input_tokens"]).unwrap_or(0).max(0) as u64;
    let output = integer_at(value, &["output_tokens"]).unwrap_or(0).max(0) as u64
        + integer_at(value, &["reasoning_output_tokens"])
            .unwrap_or(0)
            .max(0) as u64;
    let cache_read = integer_at(value, &["cached_input_tokens"]).map(|value| value.max(0) as u64);
    (input != 0 || output != 0 || cache_read.is_some()).then(|| ncs::TokenUsage {
        input,
        output,
        cache_read,
        cache_create: None,
        model: model.map(str::to_string),
    })
}

fn timestamp_at(value: &Value, path: &[&str]) -> Option<String> {
    nonempty_string_at(value, path)
        .map(str::to_string)
        .or_else(|| integer_at(value, path).map(|value| value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_rollout_without_duplicate_context_messages() {
        let directory = std::env::temp_dir().join(format!("aissence-codex-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("rollout.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"s1","cwd":"/project","model_provider":"openai"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"Fix it"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix it"}]}}"#,
                r#"{"timestamp":"2026-01-01T00:00:03Z","type":"response_item","payload":{"type":"function_call","call_id":"c1","name":"apply_patch","arguments":"{\"path\":\"x\"}"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:04Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"Done"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:05Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Fixed."}]}}"#,
                r#"{"timestamp":"2026-01-01T00:00:06Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20,"cached_input_tokens":5,"output_tokens":4,"reasoning_output_tokens":2}}}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let session = parse_file(&path).unwrap().unwrap();
        assert_eq!(session.messages_by_role(ncs::Role::User), 1);
        assert_eq!(session.messages_by_role(ncs::Role::Assistant), 2);
        assert_eq!(session.messages_by_role(ncs::Role::Tool), 1);
        assert_eq!(session.metadata.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(session.tool_uses().next().unwrap().1["path"], "x");
        assert_eq!(session.total_tokens().output, 6);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn preserves_the_model_for_each_turn_when_a_session_switches_models() {
        let directory = std::env::temp_dir().join(format!(
            "aissence-codex-model-switch-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("rollout.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"s1","cwd":"/project","model_provider":"openai"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-a"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"First"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:03Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"One"}]}}"#,
                r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2}}}}"#,
                r#"{"timestamp":"2026-01-01T00:00:05Z","type":"turn_context","payload":{"model":"gpt-b"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:06Z","type":"event_msg","payload":{"type":"user_message","message":"Second"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:07Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Two"}]}}"#,
                r#"{"timestamp":"2026-01-01T00:00:08Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20,"output_tokens":3}}}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let session = parse_file(&path).unwrap().unwrap();
        assert_eq!(session.turns.len(), 2);
        assert_eq!(
            session.turns[0]
                .token_usage
                .as_ref()
                .and_then(|usage| usage.model.as_deref()),
            Some("gpt-a")
        );
        assert_eq!(
            session.turns[1]
                .token_usage
                .as_ref()
                .and_then(|usage| usage.model.as_deref()),
            Some("gpt-b")
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn skips_internal_approval_guardian_rollouts() {
        let directory =
            std::env::temp_dir().join(format!("aissence-codex-guardian-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("rollout.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"guardian","cwd":"/project","thread_source":"subagent","source":{"subagent":{"other":"guardian"}}}}"#,
                r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"The following is the Codex agent history added since your last approval assessment."}}"#,
                r#"{"timestamp":"2026-01-01T00:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"{\"outcome\":\"allow\"}"}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        assert!(parse_file(&path).unwrap().is_none());
        let _ = std::fs::remove_dir_all(directory);
    }
}
