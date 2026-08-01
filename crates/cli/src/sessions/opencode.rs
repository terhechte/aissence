//! OpenCode session loading.
//!
//! Current OpenCode releases persist sessions in `opencode.db`; older
//! installations used one JSON file per session/message/part under `storage/`.
//! Both stores use the same logical message shape, so they converge on
//! [`normalize_session`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use normalize_chat_sessions as ncs;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

pub(super) struct LoadedSessions {
    pub sessions: Vec<ncs::Session>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct StoredSession {
    id: String,
    parent_id: Option<String>,
    directory: PathBuf,
    created: Option<i64>,
    path: PathBuf,
    messages: Vec<StoredMessage>,
}

#[derive(Debug)]
struct StoredMessage {
    id: String,
    created: Option<i64>,
    data: Value,
    parts: Vec<StoredPart>,
}

#[derive(Debug)]
struct StoredPart {
    id: String,
    created: Option<i64>,
    data: Value,
}

pub(super) fn load_sessions(folder: &Path) -> LoadedSessions {
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let explicit_database = explicit_database();

    for root in data_roots(explicit_database.as_deref()) {
        let database = explicit_database
            .clone()
            .unwrap_or_else(|| root.join("opencode.db"));
        if database.is_file() {
            load_database(&database, &folder, &mut seen, &mut sessions, &mut warnings);
        }

        // Every legacy session is also present in the database after a normal
        // OpenCode migration. `seen` makes the fallback useful without showing
        // migrated conversations twice.
        let storage = root.join("storage");
        if storage.is_dir() {
            load_legacy(&storage, &folder, &mut seen, &mut sessions, &mut warnings);
        }

        // An explicit database is a single authoritative source; do not try it
        // again for every conventional data root.
        if explicit_database.is_some() {
            break;
        }
    }

    sessions.sort_by(|a, b| {
        a.metadata
            .timestamp
            .cmp(&b.metadata.timestamp)
            .then_with(|| a.metadata.session_id.cmp(&b.metadata.session_id))
    });
    LoadedSessions { sessions, warnings }
}

fn explicit_database() -> Option<PathBuf> {
    std::env::var_os("OPENCODE_DB").map(PathBuf::from)
}

fn data_roots(explicit_database: Option<&Path>) -> Vec<PathBuf> {
    if let Some(database) = explicit_database {
        return database
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .collect();
    }

    let mut roots = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(xdg).join("opencode"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".local/share/opencode"));
        roots.push(home.join("Library/Application Support/opencode"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("opencode"));
    }
    roots.dedup();
    roots
}

fn load_database(
    path: &Path,
    folder: &Path,
    seen: &mut HashSet<String>,
    sessions: &mut Vec<ncs::Session>,
    warnings: &mut Vec<String>,
) {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = match Connection::open_with_flags(path, flags) {
        Ok(connection) => connection,
        Err(error) => {
            warnings.push(format!(
                "{}: cannot open OpenCode database: {error}",
                path.display()
            ));
            return;
        }
    };
    let _ = connection.busy_timeout(Duration::from_secs(2));

    let mut statement = match connection.prepare(
        "SELECT id, parent_id, directory, time_created FROM session ORDER BY time_created, id",
    ) {
        Ok(statement) => statement,
        Err(error) => {
            warnings.push(format!(
                "{}: cannot read OpenCode sessions: {error}",
                path.display()
            ));
            return;
        }
    };

    let rows = match statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(error) => {
            warnings.push(format!(
                "{}: cannot query OpenCode sessions: {error}",
                path.display()
            ));
            return;
        }
    };

    let metadata: Vec<_> = rows.filter_map(Result::ok).collect();
    drop(statement);
    for (id, parent_id, directory, created) in metadata {
        if seen.contains(&id) {
            continue;
        }
        if !directory_matches(Path::new(&directory), folder) {
            // Remember all database-backed ids so the legacy migration source
            // is not inspected (or warned about) for unrelated projects.
            seen.insert(id);
            continue;
        }
        match database_messages(&connection, &id) {
            Ok(messages) => {
                let stored = StoredSession {
                    id: id.clone(),
                    parent_id,
                    directory: PathBuf::from(directory),
                    created,
                    path: path.to_path_buf(),
                    messages,
                };
                sessions.push(normalize_session(stored));
                seen.insert(id);
            }
            Err(error) => warnings.push(format!("{} (session {id}): {error}", path.display())),
        }
    }
}

fn database_messages(
    connection: &Connection,
    session_id: &str,
) -> Result<Vec<StoredMessage>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, time_created, data FROM message \
             WHERE session_id = ?1 ORDER BY time_created, id",
        )
        .map_err(|error| format!("cannot prepare message query: {error}"))?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("cannot query messages: {error}"))?;
    let records: Vec<_> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read message row: {error}"))?;
    drop(statement);

    let mut messages = Vec::with_capacity(records.len());
    for (id, created, json) in records {
        let data = serde_json::from_str(&json)
            .map_err(|error| format!("invalid message {id}: {error}"))?;
        let parts = database_parts(connection, &id)?;
        messages.push(StoredMessage {
            id,
            created,
            data,
            parts,
        });
    }
    Ok(messages)
}

fn database_parts(connection: &Connection, message_id: &str) -> Result<Vec<StoredPart>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, time_created, data FROM part \
             WHERE message_id = ?1 ORDER BY time_created, id",
        )
        .map_err(|error| format!("cannot prepare part query: {error}"))?;
    let rows = statement
        .query_map(params![message_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("cannot query parts: {error}"))?;

    rows.map(|row| {
        let (id, created, json) = row.map_err(|error| format!("cannot read part row: {error}"))?;
        let data =
            serde_json::from_str(&json).map_err(|error| format!("invalid part {id}: {error}"))?;
        Ok(StoredPart { id, created, data })
    })
    .collect()
}

fn load_legacy(
    storage: &Path,
    folder: &Path,
    seen: &mut HashSet<String>,
    sessions: &mut Vec<ncs::Session>,
    warnings: &mut Vec<String>,
) {
    let mut session_files = Vec::new();
    collect_json_files(&storage.join("session"), &mut session_files);
    session_files.sort();

    for path in session_files {
        let data = match read_json(&path) {
            Ok(data) => data,
            Err(error) => {
                warnings.push(error);
                continue;
            }
        };
        let Some(id) = string_at(&data, &["id"]) else {
            warnings.push(format!("{}: OpenCode session has no id", path.display()));
            continue;
        };
        if seen.contains(id) {
            continue;
        }
        let Some(directory) = string_at(&data, &["directory"]) else {
            // Some old global sessions have no project directory and therefore
            // cannot be attributed to the folder being analyzed.
            continue;
        };
        if !directory_matches(Path::new(directory), folder) {
            continue;
        }

        match legacy_messages(storage, id) {
            Ok(messages) => {
                let id = id.to_string();
                let stored = StoredSession {
                    id: id.clone(),
                    parent_id: string_at(&data, &["parentID"]).map(str::to_string),
                    directory: PathBuf::from(directory),
                    created: integer_at(&data, &["time", "created"]),
                    path,
                    messages,
                };
                sessions.push(normalize_session(stored));
                seen.insert(id);
            }
            Err(error) => warnings.push(error),
        }
    }
}

fn legacy_messages(storage: &Path, session_id: &str) -> Result<Vec<StoredMessage>, String> {
    let mut paths = Vec::new();
    collect_json_files(&storage.join("message").join(session_id), &mut paths);
    let mut messages = Vec::with_capacity(paths.len());
    for path in paths {
        let data = read_json(&path)?;
        let id = string_at(&data, &["id"])
            .map(str::to_string)
            .or_else(|| path.file_stem()?.to_str().map(str::to_string))
            .ok_or_else(|| format!("{}: OpenCode message has no id", path.display()))?;
        messages.push(StoredMessage {
            created: integer_at(&data, &["time", "created"]),
            parts: legacy_parts(storage, &id)?,
            id,
            data,
        });
    }
    messages.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id)));
    Ok(messages)
}

fn legacy_parts(storage: &Path, message_id: &str) -> Result<Vec<StoredPart>, String> {
    let mut paths = Vec::new();
    collect_json_files(&storage.join("part").join(message_id), &mut paths);
    let mut parts = Vec::with_capacity(paths.len());
    for path in paths {
        let data = read_json(&path)?;
        let id = string_at(&data, &["id"])
            .map(str::to_string)
            .or_else(|| path.file_stem()?.to_str().map(str::to_string))
            .ok_or_else(|| format!("{}: OpenCode part has no id", path.display()))?;
        parts.push(StoredPart {
            created: integer_at(&data, &["time", "start"]),
            id,
            data,
        });
    }
    parts.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id)));
    Ok(parts)
}

fn normalize_session(stored: StoredSession) -> ncs::Session {
    let mut session = ncs::Session::new(stored.path, "opencode");
    session.metadata.session_id = Some(stored.id);
    session.metadata.timestamp = stored.created.map(|value| value.to_string());
    session.metadata.project = Some(stored.directory.to_string_lossy().into_owned());
    session.parent_id = stored.parent_id;

    let mut current = ncs::Turn::default();
    for message in stored.messages {
        if bool_at(&message.data, &["summary"]) == Some(true) {
            continue;
        }
        let Some(role) = role(&message.data) else {
            continue;
        };

        if role == ncs::Role::User && !current.messages.is_empty() {
            session.turns.push(std::mem::take(&mut current));
        }

        if session.metadata.provider.is_none() {
            session.metadata.provider = message_provider(&message.data).map(str::to_string);
        }
        if session.metadata.model.is_none() {
            session.metadata.model = message_model(&message.data).map(str::to_string);
        }
        if session.subagent_type.is_none() {
            session.subagent_type = string_at(&message.data, &["agent"])
                .or_else(|| string_at(&message.data, &["mode"]))
                .map(str::to_string);
        }

        let timestamp = integer_at(&message.data, &["time", "created"])
            .or(message.created)
            .map(|value| value.to_string());
        let mut content = Vec::new();
        let mut tool_results = Vec::new();

        for part in message.parts {
            match string_at(&part.data, &["type"]).unwrap_or("") {
                "text"
                    if bool_at(&part.data, &["synthetic"]) != Some(true)
                        && bool_at(&part.data, &["ignored"]) != Some(true) =>
                {
                    if let Some(text) = nonempty_string_at(&part.data, &["text"]) {
                        content.push(ncs::ContentBlock::Text {
                            text: text.to_string(),
                        });
                    }
                }
                "reasoning" => {
                    if let Some(text) = nonempty_string_at(&part.data, &["text"]) {
                        content.push(ncs::ContentBlock::Thinking {
                            text: text.to_string(),
                        });
                    }
                }
                "tool" => {
                    let call_id = string_at(&part.data, &["callID"])
                        .unwrap_or(&part.id)
                        .to_string();
                    let name = string_at(&part.data, &["tool"])
                        .unwrap_or("unknown")
                        .to_string();
                    let input = value_at(&part.data, &["state", "input"])
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Default::default()));
                    content.push(ncs::ContentBlock::ToolUse {
                        id: call_id.clone(),
                        name,
                        input,
                    });

                    let status = string_at(&part.data, &["state", "status"]);
                    let result = match status {
                        Some("completed") => string_at(&part.data, &["state", "output"])
                            .map(|output| (output.to_string(), false)),
                        Some("error") => string_at(&part.data, &["state", "error"])
                            .map(|error| (error.to_string(), true)),
                        _ => None,
                    };
                    if let Some((result, is_error)) = result {
                        let result_timestamp = integer_at(&part.data, &["state", "time", "end"])
                            .or(part.created)
                            .map(|value| value.to_string())
                            .or_else(|| timestamp.clone());
                        tool_results.push(ncs::Message {
                            role: ncs::Role::Tool,
                            content: vec![ncs::ContentBlock::ToolResult {
                                tool_use_id: call_id,
                                content: result,
                                is_error,
                            }],
                            timestamp: result_timestamp,
                        });
                    }
                }
                "subtask" => {
                    let mut input = part.data.clone();
                    if let Some(object) = input.as_object_mut() {
                        object.remove("type");
                    }
                    content.push(ncs::ContentBlock::ToolUse {
                        id: part.id,
                        name: "subtask".to_string(),
                        input,
                    });
                }
                _ => {}
            }
        }

        if !content.is_empty() {
            current.messages.push(ncs::Message {
                role,
                content,
                timestamp,
            });
        }
        current.messages.extend(tool_results);

        if let Some(usage) = token_usage(&message.data) {
            let total = current.token_usage.get_or_insert_with(Default::default);
            total.input += usage.input;
            total.output += usage.output;
            add_optional(&mut total.cache_read, usage.cache_read);
            add_optional(&mut total.cache_create, usage.cache_create);
            if total.model.is_none() {
                total.model = usage.model;
            }
        }
    }
    if !current.messages.is_empty() {
        session.turns.push(current);
    }
    session
}

fn token_usage(data: &Value) -> Option<ncs::TokenUsage> {
    let tokens = value_at(data, &["tokens"])?;
    Some(ncs::TokenUsage {
        input: integer_at(tokens, &["input"]).unwrap_or(0).max(0) as u64,
        output: integer_at(tokens, &["output"]).unwrap_or(0).max(0) as u64,
        cache_read: integer_at(tokens, &["cache", "read"]).map(|value| value.max(0) as u64),
        cache_create: integer_at(tokens, &["cache", "write"]).map(|value| value.max(0) as u64),
        model: message_model(data).map(str::to_string),
    })
}

fn add_optional(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target.get_or_insert(0) += value;
    }
}

fn role(data: &Value) -> Option<ncs::Role> {
    match string_at(data, &["role"])? {
        "user" => Some(ncs::Role::User),
        "assistant" => Some(ncs::Role::Assistant),
        "system" => Some(ncs::Role::System),
        "tool" => Some(ncs::Role::Tool),
        _ => None,
    }
}

fn message_provider(data: &Value) -> Option<&str> {
    string_at(data, &["providerID"]).or_else(|| string_at(data, &["model", "providerID"]))
}

fn message_model(data: &Value) -> Option<&str> {
    string_at(data, &["modelID"]).or_else(|| string_at(data, &["model", "modelID"]))
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

fn collect_json_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, output);
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            output.push(path);
        }
    }
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
        let path =
            std::env::temp_dir().join(format!("aissence-opencode-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loads_sqlite_messages_parts_and_project_scope() {
        let root = temp_dir("sqlite");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let database = root.join("opencode.db");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT, time_created INTEGER);\
                 CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);\
                 CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session VALUES (?1, NULL, ?2, 1000)",
                params!["ses_test", project.to_string_lossy()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session VALUES ('ses_other', NULL, '/unrelated', 1000)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message VALUES ('msg_user', 'ses_test', 1100, ?1)",
                params![r#"{"role":"user","time":{"created":1100},"agent":"build","model":{"providerID":"openrouter","modelID":"example/model"}}"#],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_user', 'msg_user', 'ses_test', 1100, ?1)",
                params![r#"{"type":"text","text":"Please change it"}"#],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message VALUES ('msg_ai', 'ses_test', 1200, ?1)",
                params![r#"{"role":"assistant","time":{"created":1200},"providerID":"openrouter","modelID":"example/model","agent":"build","tokens":{"input":10,"output":4,"cache":{"read":2,"write":1}}}"#],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_think', 'msg_ai', 'ses_test', 1200, ?1)",
                params![r#"{"type":"reasoning","text":"Working it out"}"#],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_tool', 'msg_ai', 'ses_test', 1201, ?1)",
                params![r#"{"type":"tool","callID":"call_1","tool":"write","state":{"status":"completed","input":{"filePath":"src/lib.rs","content":"new"},"output":"Done","time":{"end":1250}}}"#],
            )
            .unwrap();
        drop(connection);

        let mut seen = HashSet::new();
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        load_database(&database, &project, &mut seen, &mut sessions, &mut warnings);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.format, "opencode");
        assert_eq!(session.metadata.provider.as_deref(), Some("openrouter"));
        assert_eq!(session.metadata.model.as_deref(), Some("example/model"));
        assert_eq!(session.message_count(), 3); // user, assistant, tool result
        assert_eq!(session.tool_uses().next().unwrap().0, "write");
        assert_eq!(session.tool_results().next(), Some(("Done", false)));
        assert_eq!(session.total_tokens().input, 10);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_sessions_are_loaded_and_seen_ids_are_not_duplicated() {
        let root = temp_dir("legacy");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let storage = root.join("storage");
        let session_dir = storage.join("session/project_hash");
        let message_dir = storage.join("message/ses_legacy");
        let part_dir = storage.join("part/msg_legacy");
        for directory in [&session_dir, &message_dir, &part_dir] {
            std::fs::create_dir_all(directory).unwrap();
        }
        std::fs::write(
            session_dir.join("ses_legacy.json"),
            format!(
                r#"{{"id":"ses_legacy","directory":{},"time":{{"created":1000}}}}"#,
                serde_json::to_string(&project.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        std::fs::write(
            message_dir.join("msg_legacy.json"),
            r#"{"id":"msg_legacy","role":"user","time":{"created":1100},"agent":"build","model":{"providerID":"test","modelID":"model"}}"#,
        )
        .unwrap();
        std::fs::write(
            part_dir.join("prt_legacy.json"),
            r#"{"id":"prt_legacy","type":"text","text":"Legacy prompt"}"#,
        )
        .unwrap();

        let mut seen = HashSet::new();
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        load_legacy(&storage, &project, &mut seen, &mut sessions, &mut warnings);
        load_legacy(&storage, &project, &mut seen, &mut sessions, &mut warnings);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message_count(), 1);
        assert_eq!(sessions[0].turns[0].messages[0].content.len(), 1);

        std::fs::remove_dir_all(root).unwrap();
    }
}
