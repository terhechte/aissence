//! Historical local Amp thread parser.
//!
//! Modern Amp stores complete threads server-side, but older releases wrote
//! `T-*.json` files under `~/.local/share/amp/threads`. Those files remain
//! useful and contain enough workspace URI metadata for exact project scoping.

use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

use super::common::{
    bool_at, collect_files, directory_matches, file_uri_path, integer_at, nonempty_string_at,
    string_at, text_or_json, value_at,
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
        collect_files(&root, &["json"], &mut files);
    }
    files.sort();
    files.dedup();

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    for path in files {
        if !path
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with("T-"))
        {
            continue;
        }
        match parse_file(&path) {
            Ok(Some(session))
                if session
                    .metadata
                    .project
                    .as_deref()
                    .is_some_and(|project| directory_matches(Path::new(project), &folder)) =>
            {
                sessions.push(session)
            }
            Ok(_) => {}
            Err(error) => warnings.push(error),
        }
    }
    LoadedSessions { sessions, warnings }
}

fn roots() -> Vec<PathBuf> {
    if let Some(root) = std::env::var_os("AMP_DIR") {
        return vec![PathBuf::from(root)];
    }
    let mut roots = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(xdg).join("amp/threads"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/amp/threads"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("amp/threads"));
    }
    roots
}

fn parse_file(path: &Path) -> Result<Option<ncs::Session>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let root: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{}: invalid Amp thread: {error}", path.display()))?;
    if integer_at(&root, &["v"]) != Some(1) && root.get("messages").is_none() {
        return Ok(None);
    }

    let Some(project) = project_path(&root) else {
        return Ok(None);
    };
    let mut session = ncs::Session::new(path.to_path_buf(), "amp");
    session.metadata.session_id = string_at(&root, &["id"])
        .map(str::to_string)
        .or_else(|| path.file_stem()?.to_str().map(str::to_string));
    session.metadata.timestamp = timestamp(&root);
    session.metadata.project = Some(project.to_string_lossy().into_owned());

    let mut current = ncs::Turn::default();
    for message in root
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let role = string_at(message, &["role"]).unwrap_or("");
        if role == "user" && has_user_text(message) && !current.messages.is_empty() {
            session.turns.push(std::mem::take(&mut current));
        }

        let mut regular = Vec::new();
        let mut results = Vec::new();
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match string_at(block, &["type"]).unwrap_or("") {
                "text" => {
                    if let Some(text) = nonempty_string_at(block, &["text"]) {
                        regular.push(ncs::ContentBlock::Text {
                            text: text.to_string(),
                        });
                    }
                }
                "thinking" => {
                    if let Some(text) = nonempty_string_at(block, &["thinking"])
                        .or_else(|| nonempty_string_at(block, &["text"]))
                    {
                        regular.push(ncs::ContentBlock::Thinking {
                            text: text.to_string(),
                        });
                    }
                }
                "tool_use" => regular.push(ncs::ContentBlock::ToolUse {
                    id: string_at(block, &["id"]).unwrap_or("").to_string(),
                    name: string_at(block, &["name"]).unwrap_or("unknown").to_string(),
                    input: value_at(block, &["input"]).cloned().unwrap_or(Value::Null),
                }),
                "tool_result" => {
                    if let Some(result) = amp_tool_result(block) {
                        results.push(result);
                    }
                }
                _ => {}
            }
        }

        let message_time = timestamp(message);
        let normalized_role = match role {
            "user" => Some(ncs::Role::User),
            "assistant" => Some(ncs::Role::Assistant),
            "system" => Some(ncs::Role::System),
            _ => None,
        };
        if !regular.is_empty()
            && let Some(role) = normalized_role
        {
            current.messages.push(ncs::Message {
                role,
                content: regular,
                timestamp: message_time.clone(),
            });
        }
        for (tool_use_id, content, is_error) in results {
            current.messages.push(ncs::Message {
                role: ncs::Role::Tool,
                content: vec![ncs::ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                }],
                timestamp: message_time.clone(),
            });
        }

        if session.metadata.model.is_none() && role == "assistant" {
            session.metadata.model = string_at(message, &["model"]).map(str::to_string);
            session.metadata.provider = string_at(message, &["provider"]).map(str::to_string);
        }
    }
    if !current.messages.is_empty() {
        session.turns.push(current);
    }
    Ok((!session.turns.is_empty()).then_some(session))
}

fn project_path(root: &Value) -> Option<PathBuf> {
    let trees = value_at(root, &["env", "initial", "trees"])?.as_array()?;
    trees.iter().find_map(|tree| {
        nonempty_string_at(tree, &["uri"])
            .map(file_uri_path)
            .or_else(|| nonempty_string_at(tree, &["path"]).map(PathBuf::from))
    })
}

fn has_user_text(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|block| {
                string_at(block, &["type"]) == Some("text")
                    && nonempty_string_at(block, &["text"]).is_some()
            })
        })
}

fn amp_tool_result(block: &Value) -> Option<(String, String, bool)> {
    if let Some(id) = nonempty_string_at(block, &["tool_use_id"]) {
        return Some((
            id.to_string(),
            text_or_json(block.get("content")),
            bool_at(block, &["is_error"]).unwrap_or(false),
        ));
    }

    let id = nonempty_string_at(block, &["toolUseID"])?;
    let status = string_at(block, &["run", "status"]).unwrap_or("");
    let (content, is_error) = match status {
        "error" => (
            nonempty_string_at(block, &["run", "error", "message"])
                .unwrap_or("[unknown error]")
                .to_string(),
            true,
        ),
        "cancelled" => ("[cancelled]".to_string(), true),
        _ if value_at(block, &["run", "result"]).is_some() => {
            (serialize_result(value_at(block, &["run", "result"])), false)
        }
        _ => return None,
    };
    Some((id.to_string(), content, is_error))
}

fn serialize_result(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        Value::Object(object) => {
            for key in ["output", "content", "diff"] {
                if let Some(value) = object.get(key) {
                    return text_or_json(Some(value));
                }
            }
            if let Some(success) = object.get("success").and_then(Value::as_bool) {
                return if success { "success" } else { "failed" }.to_string();
            }
            value.to_string()
        }
        Value::Array(items) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => text_or_json(Some(value)),
    }
}

fn timestamp(value: &Value) -> Option<String> {
    nonempty_string_at(value, &["timestamp"])
        .or_else(|| nonempty_string_at(value, &["createdAt"]))
        .map(str::to_string)
        .or_else(|| integer_at(value, &["created"]).map(|value| value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_historical_thread_and_amp_tool_results() {
        let directory = std::env::temp_dir().join(format!("aissence-amp-{}", std::process::id()));
        let project = directory.join("my project");
        std::fs::create_dir_all(&project).unwrap();
        let path = directory.join("T-test.json");
        let uri = format!("file://{}", project.to_string_lossy().replace(' ', "%20"));
        let data = serde_json::json!({
            "v": 1, "id": "T-test", "created": 1767225600000_i64,
            "env": {"initial": {"trees": [{"uri": uri}]}},
            "messages": [
                {"role":"user","content":[{"type":"text","text":"Read it"}]},
                {"role":"assistant","content":[{"type":"tool_use","id":"tu1","name":"Read","input":{"path":"x"}}]},
                {"role":"user","content":[{"type":"tool_result","toolUseID":"tu1","run":{"status":"done","result":{"content":"hello"}}}]}
            ]
        });
        std::fs::write(&path, data.to_string()).unwrap();

        let session = parse_file(&path).unwrap().unwrap();
        assert_eq!(
            Path::new(session.metadata.project.as_deref().unwrap()),
            project
        );
        assert_eq!(session.tool_uses().next().unwrap().0, "Read");
        assert_eq!(session.tool_results().next(), Some(("hello", false)));
        assert_eq!(session.message_count(), 3);
        let _ = std::fs::remove_dir_all(directory);
    }
}
