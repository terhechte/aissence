//! VS Code / GitHub Copilot chat-session parser.
//!
//! Sessions are scoped by the containing `workspaceStorage/<hash>/workspace.json`.
//! VS Code historically wrote full JSON snapshots and now also writes JSONL
//! mutation logs; both normalize through the same snapshot parser.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use normalize_chat_sessions as ncs;
use serde_json::Value;

use super::common::{
    directory_matches, file_uri_path, integer_at, nonempty_string_at, read_json, string_at,
    value_at,
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
        discover_root(&root, &folder, &mut files);
    }
    files.sort();
    files.dedup();

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    for path in files {
        match parse_file(&path, &folder) {
            Ok(Some(session)) => sessions.push(session),
            Ok(None) => {}
            Err(error) => warnings.push(error),
        }
    }
    LoadedSessions { sessions, warnings }
}

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = std::env::var_os("VSCODE_USER_DATA_DIR") {
        roots.push(PathBuf::from(root).join("User/workspaceStorage"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join("Library/Application Support/Code/User/workspaceStorage"));
        roots.push(home.join("Library/Application Support/Code - Insiders/User/workspaceStorage"));
        roots.push(home.join(".config/Code/User/workspaceStorage"));
        roots.push(home.join(".config/Code - Insiders/User/workspaceStorage"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let appdata = PathBuf::from(appdata);
        roots.push(appdata.join("Code/User/workspaceStorage"));
        roots.push(appdata.join("Code - Insiders/User/workspaceStorage"));
    }
    roots.sort();
    roots.dedup();
    roots
}

fn discover_root(root: &Path, folder: &Path, output: &mut Vec<PathBuf>) {
    let Ok(workspaces) = std::fs::read_dir(root) else {
        return;
    };
    for workspace in workspaces.flatten() {
        let workspace = workspace.path();
        if !workspace.is_dir() || !workspace_matches(&workspace, folder) {
            continue;
        }
        let chats = workspace.join("chatSessions");
        let Ok(entries) = std::fs::read_dir(chats) else {
            continue;
        };
        let mut by_stem: BTreeMap<String, PathBuf> = BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let extension = path.extension().and_then(|value| value.to_str());
            if !matches!(extension, Some("json") | Some("jsonl")) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let replace = extension == Some("jsonl")
                || !by_stem.get(stem).is_some_and(|existing| {
                    existing.extension().and_then(|v| v.to_str()) == Some("jsonl")
                });
            if replace {
                by_stem.insert(stem.to_string(), path);
            }
        }
        output.extend(by_stem.into_values());
    }
}

fn workspace_matches(hash_dir: &Path, folder: &Path) -> bool {
    let Ok(workspace) = read_json(&hash_dir.join("workspace.json")) else {
        return false;
    };
    if let Some(uri) = nonempty_string_at(&workspace, &["folder"]) {
        return directory_matches(&file_uri_path(uri), folder);
    }
    let Some(uri) = nonempty_string_at(&workspace, &["workspace"]) else {
        return false;
    };
    let manifest = file_uri_path(uri);
    if directory_matches(&manifest, folder) {
        return true;
    }
    let Ok(data) = read_json(&manifest) else {
        return false;
    };
    data.get("folders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|entry| {
            let path = nonempty_string_at(entry, &["uri"])
                .map(file_uri_path)
                .or_else(|| {
                    nonempty_string_at(entry, &["path"]).map(|path| {
                        if Path::new(path).is_absolute() {
                            PathBuf::from(path)
                        } else {
                            manifest.parent().unwrap_or(Path::new("")).join(path)
                        }
                    })
                });
            path.is_some_and(|path| directory_matches(&path, folder))
        })
}

fn parse_file(path: &Path, folder: &Path) -> Result<Option<ncs::Session>, String> {
    let snapshot = if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
        reconstruct_jsonl(path)?
    } else {
        Some(read_json(path)?)
    };
    let Some(root) = snapshot else {
        return Ok(None);
    };
    let Some(requests) = root.get("requests").and_then(Value::as_array) else {
        return Ok(None);
    };

    let mut session = ncs::Session::new(path.to_path_buf(), "vscode-copilot");
    session.metadata.session_id = string_at(&root, &["sessionId"])
        .map(str::to_string)
        .or_else(|| path.file_stem()?.to_str().map(str::to_string));
    session.metadata.timestamp = timestamp_at(&root, &["creationDate"])
        .or_else(|| timestamp_at(&root, &["lastMessageDate"]));
    session.metadata.project = Some(folder.to_string_lossy().into_owned());
    session.metadata.provider = Some("github-copilot".to_string());

    for request in requests {
        let timestamp = timestamp_at(request, &["timestamp"]);
        let mut turn = ncs::Turn::default();
        if let Some(text) = nonempty_string_at(request, &["message", "text"]) {
            turn.messages.push(ncs::Message {
                role: ncs::Role::User,
                content: vec![ncs::ContentBlock::Text {
                    text: text.to_string(),
                }],
                timestamp: timestamp.clone(),
            });
        }

        let blocks = response_blocks(request);
        if !blocks.is_empty() {
            turn.messages.push(ncs::Message {
                role: ncs::Role::Assistant,
                content: blocks,
                timestamp: timestamp.clone(),
            });
        }
        if let Some(usage) = request_usage(request) {
            if session.metadata.model.is_none() {
                session.metadata.model = usage.model.clone().or_else(|| request_model(request));
            }
            turn.token_usage = Some(usage);
        } else if session.metadata.model.is_none() {
            session.metadata.model = request_model(request);
        }
        if !turn.messages.is_empty() {
            session.turns.push(turn);
        }
    }
    Ok((!session.turns.is_empty()).then_some(session))
}

fn response_blocks(request: &Value) -> Vec<ncs::ContentBlock> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    for item in request
        .get("response")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match string_at(item, &["kind"]).unwrap_or("") {
            "toolInvocationSerialized" => {
                flush_text(&mut text, &mut blocks);
                let name = string_at(item, &["toolId"])
                    .or_else(|| string_at(item, &["toolName"]))
                    .unwrap_or("unknown");
                let id = string_at(item, &["toolCallId"]).unwrap_or("").to_string();
                let mut input = serde_json::Map::new();
                if let Some(message) = invocation_text(item) {
                    input.insert("message".to_string(), Value::String(message));
                }
                if let Some(command) = nonempty_string_at(item, &["toolSpecificData", "command"]) {
                    input.insert("command".to_string(), Value::String(command.to_string()));
                }
                blocks.push(ncs::ContentBlock::ToolUse {
                    id,
                    name: name.to_string(),
                    input: Value::Object(input),
                });
            }
            "prepareToolInvocation"
            | "inlineReference"
            | "undoStop"
            | "codeblockUri"
            | "textEditGroup" => {}
            _ => {
                if let Some(value) = nonempty_string_at(item, &["value"]) {
                    text.push_str(value);
                }
            }
        }
    }
    flush_text(&mut text, &mut blocks);
    blocks
}

fn flush_text(text: &mut String, blocks: &mut Vec<ncs::ContentBlock>) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        blocks.push(ncs::ContentBlock::Text {
            text: trimmed.to_string(),
        });
    }
    text.clear();
}

fn invocation_text(item: &Value) -> Option<String> {
    for key in ["pastTenseMessage", "invocationMessage"] {
        match item.get(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => return Some(text.clone()),
            Some(Value::Object(object)) => {
                if let Some(text) = object.get("value").and_then(Value::as_str)
                    && !text.trim().is_empty()
                {
                    return Some(text.to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn request_usage(request: &Value) -> Option<ncs::TokenUsage> {
    let metadata = value_at(request, &["result", "metadata"])?;
    let input = integer_at(metadata, &["promptTokens"]).unwrap_or(0).max(0) as u64;
    let output = integer_at(metadata, &["outputTokens"]).unwrap_or(0).max(0) as u64;
    (input != 0 || output != 0).then(|| ncs::TokenUsage {
        input,
        output,
        cache_read: None,
        cache_create: None,
        model: nonempty_string_at(metadata, &["resolvedModel"])
            .map(str::to_string)
            .or_else(|| request_model(request)),
    })
}

fn request_model(request: &Value) -> Option<String> {
    nonempty_string_at(request, &["modelId"])
        .map(|model| model.strip_prefix("copilot/").unwrap_or(model).to_string())
}

fn reconstruct_jsonl(path: &Path) -> Result<Option<Value>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut state = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(operation) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match integer_at(&operation, &["kind"]) {
            Some(0) => state = operation.get("v").cloned(),
            Some(1) => {
                if let (Some(state), Some(keys), Some(value)) = (
                    state.as_mut(),
                    operation.get("k").and_then(Value::as_array),
                    operation.get("v"),
                ) {
                    set_at(state, keys, value.clone());
                }
            }
            Some(2) => {
                if let (Some(state), Some(keys), Some(items)) = (
                    state.as_mut(),
                    operation.get("k").and_then(Value::as_array),
                    operation.get("v").and_then(Value::as_array),
                ) {
                    push_at(
                        state,
                        keys,
                        items.clone(),
                        integer_at(&operation, &["i"])
                            .and_then(|value| usize::try_from(value).ok()),
                    );
                }
            }
            Some(3) => {
                if let (Some(state), Some(keys)) =
                    (state.as_mut(), operation.get("k").and_then(Value::as_array))
                {
                    delete_at(state, keys);
                }
            }
            _ => {}
        }
    }
    Ok(state)
}

fn descend_mut<'a>(state: &'a mut Value, keys: &[Value]) -> Option<&'a mut Value> {
    let mut current = state;
    for key in keys {
        current = if let Some(key) = key.as_str() {
            current.get_mut(key)?
        } else {
            current.get_mut(key.as_u64()? as usize)?
        };
    }
    Some(current)
}

fn set_at(state: &mut Value, keys: &[Value], value: Value) {
    let Some((last, parents)) = keys.split_last() else {
        *state = value;
        return;
    };
    let Some(parent) = descend_mut(state, parents) else {
        return;
    };
    if let Some(key) = last.as_str() {
        if let Some(object) = parent.as_object_mut() {
            object.insert(key.to_string(), value);
        }
    } else if let (Some(array), Some(index)) = (parent.as_array_mut(), last.as_u64())
        && let Some(slot) = array.get_mut(index as usize)
    {
        *slot = value;
    }
}

fn push_at(state: &mut Value, keys: &[Value], items: Vec<Value>, index: Option<usize>) {
    let Some(target) = descend_mut(state, keys) else {
        return;
    };
    let Some(array) = target.as_array_mut() else {
        return;
    };
    if let Some(index) = index {
        let index = index.min(array.len());
        let end = (index + items.len()).min(array.len());
        array.splice(index..end, items);
    } else {
        array.extend(items);
    }
}

fn delete_at(state: &mut Value, keys: &[Value]) {
    let Some((last, parents)) = keys.split_last() else {
        return;
    };
    let Some(parent) = descend_mut(state, parents) else {
        return;
    };
    if let (Some(object), Some(key)) = (parent.as_object_mut(), last.as_str()) {
        object.remove(key);
    }
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
    fn parses_jsonl_mutations_tools_and_tokens() {
        let directory =
            std::env::temp_dir().join(format!("aissence-vscode-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("session.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"kind":0,"v":{"version":3,"sessionId":"s1","creationDate":1767225600000,"requests":[]}}"#,
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r1","timestamp":1767225601000,"message":{"text":"Run tests"},"response":[{"kind":"toolInvocationSerialized","toolId":"copilot_runInTerminal","toolCallId":"c1","toolSpecificData":{"command":"cargo test"}},{"value":"Done."}],"modelId":"copilot/gpt-5","result":{"metadata":{"promptTokens":30,"outputTokens":5,"resolvedModel":"gpt-5"}}}]}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let session = parse_file(&path, Path::new("/project")).unwrap().unwrap();
        assert_eq!(session.message_count(), 2);
        assert_eq!(
            session.tool_uses().next().unwrap().1["command"],
            "cargo test"
        );
        assert_eq!(session.total_tokens().input, 30);
        assert_eq!(session.metadata.model.as_deref(), Some("gpt-5"));
        let _ = std::fs::remove_dir_all(directory);
    }
}
