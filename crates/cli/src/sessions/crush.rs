//! Crush session parser.
//!
//! Crush keeps one SQLite database per project at `.crush/crush.db`. Messages
//! contain a JSON array of typed `{type,data}` content parts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use normalize_chat_sessions as ncs;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use super::common::{bool_at, integer_at, nonempty_string_at, string_at, text_or_json, value_at};

pub(super) struct LoadedSessions {
    pub sessions: Vec<ncs::Session>,
    pub warnings: Vec<String>,
}

pub(super) fn load_sessions(folder: &Path) -> LoadedSessions {
    let database = std::env::var_os("CRUSH_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| folder.join(".crush/crush.db"));
    if !database.is_file() {
        return LoadedSessions {
            sessions: Vec::new(),
            warnings: Vec::new(),
        };
    }

    match load_database(&database, folder) {
        Ok(sessions) => LoadedSessions {
            sessions,
            warnings: Vec::new(),
        },
        Err(error) => LoadedSessions {
            sessions: Vec::new(),
            warnings: vec![format!("{}: {error}", database.display())],
        },
    }
}

fn load_database(path: &Path, folder: &Path) -> Result<Vec<ncs::Session>, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|error| format!("open Crush DB: {error}"))?;
    let _ = connection.busy_timeout(Duration::from_secs(2));

    let has_provider = has_column(&connection, "messages", "provider");
    let has_summary = has_column(&connection, "messages", "is_summary_message");
    let query = format!(
        "SELECT id, session_id, role, parts, model, {}, created_at, {} \
         FROM messages ORDER BY session_id, created_at, id",
        if has_provider { "provider" } else { "NULL" },
        if has_summary {
            "is_summary_message"
        } else {
            "0"
        },
    );

    let mut session_statement = connection
        .prepare(
            "SELECT id, parent_session_id, prompt_tokens, completion_tokens, created_at \
             FROM sessions ORDER BY created_at, id",
        )
        .map_err(|error| format!("read Crush sessions: {error}"))?;
    let session_rows = session_statement
        .query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                input: row.get::<_, i64>(2)?.max(0) as u64,
                output: row.get::<_, i64>(3)?.max(0) as u64,
                created: row.get(4)?,
            })
        })
        .map_err(|error| format!("query Crush sessions: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("decode Crush session: {error}"))?;
    drop(session_statement);

    let mut message_statement = connection
        .prepare(&query)
        .map_err(|error| format!("read Crush messages: {error}"))?;
    let message_rows = message_statement
        .query_map([], |row| {
            Ok(MessageRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                parts: row.get(3)?,
                model: row.get(4)?,
                provider: row.get(5)?,
                created: row.get(6)?,
                summary: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|error| format!("query Crush messages: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("decode Crush message: {error}"))?;

    let project = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let mut sessions = Vec::new();
    for stored in session_rows {
        let messages = message_rows
            .iter()
            .filter(|message| message.session_id == stored.id && !message.summary);
        let mut session = ncs::Session::new(path.to_path_buf(), "crush");
        session.metadata.session_id = Some(stored.id.clone());
        session.metadata.timestamp = Some(stored.created.to_string());
        session.metadata.project = Some(project.clone());
        session.parent_id = stored.parent_id;

        let mut current = ncs::Turn::default();
        for message in messages {
            let parts: Vec<Value> = match serde_json::from_str(&message.parts) {
                Ok(parts) => parts,
                Err(_) => continue,
            };
            if message.role == "user" && !current.messages.is_empty() {
                session.turns.push(std::mem::take(&mut current));
            }
            if session.metadata.model.is_none() {
                session.metadata.model = message.model.clone();
            }
            if session.metadata.provider.is_none() {
                session.metadata.provider = message.provider.clone();
            }

            let timestamp = Some(message.created.to_string());
            let mut content = Vec::new();
            let mut results = Vec::new();
            for (part_index, part) in parts.iter().enumerate() {
                let data = part.get("data").unwrap_or(part);
                match string_at(part, &["type"]).unwrap_or("") {
                    "text" => {
                        if let Some(text) = nonempty_string_at(data, &["text"]) {
                            content.push(ncs::ContentBlock::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                    "reasoning" => {
                        if let Some(text) = nonempty_string_at(data, &["thinking"]) {
                            content.push(ncs::ContentBlock::Thinking {
                                text: text.to_string(),
                            });
                        }
                    }
                    "tool_call" => {
                        let input = string_at(data, &["input"])
                            .and_then(|text| serde_json::from_str(text).ok())
                            .or_else(|| value_at(data, &["input"]).cloned())
                            .unwrap_or(Value::Null);
                        content.push(ncs::ContentBlock::ToolUse {
                            id: string_at(data, &["id"]).unwrap_or("").to_string(),
                            name: string_at(data, &["name"]).unwrap_or("unknown").to_string(),
                            input,
                        });
                    }
                    "tool_result" => results.push((
                        string_at(data, &["tool_call_id"]).unwrap_or("").to_string(),
                        nonempty_string_at(data, &["content"])
                            .or_else(|| nonempty_string_at(data, &["data"]))
                            .unwrap_or("")
                            .to_string(),
                        bool_at(data, &["is_error"]).unwrap_or(false),
                    )),
                    "shell_command" => {
                        let id = format!("{}-shell-{part_index}", message.id);
                        content.push(ncs::ContentBlock::ToolUse {
                            id: id.clone(),
                            name: "shell".to_string(),
                            input: serde_json::json!({
                                "command": string_at(data, &["command"]).unwrap_or("")
                            }),
                        });
                        results.push((
                            id,
                            text_or_json(data.get("output")),
                            integer_at(data, &["exit_code"]).unwrap_or(0) != 0,
                        ));
                    }
                    _ => {}
                }
            }

            if !content.is_empty()
                && let Some(role) = role(&message.role)
            {
                current.messages.push(ncs::Message {
                    role,
                    content,
                    timestamp: timestamp.clone(),
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
                    timestamp: timestamp.clone(),
                });
            }
        }
        if !current.messages.is_empty() {
            session.turns.push(current);
        }
        if let Some(last) = session.turns.last_mut()
            && (stored.input != 0 || stored.output != 0)
        {
            last.token_usage = Some(ncs::TokenUsage {
                input: stored.input,
                output: stored.output,
                cache_read: None,
                cache_create: None,
                model: session.metadata.model.clone(),
            });
        }
        if !session.turns.is_empty() {
            sessions.push(session);
        }
    }
    Ok(sessions)
}

fn has_column(connection: &Connection, table: &str, column: &str) -> bool {
    connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, column],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0
}

fn role(role: &str) -> Option<ncs::Role> {
    match role {
        "user" => Some(ncs::Role::User),
        "assistant" => Some(ncs::Role::Assistant),
        "system" => Some(ncs::Role::System),
        "tool" => Some(ncs::Role::Tool),
        _ => None,
    }
}

struct SessionRow {
    id: String,
    parent_id: Option<String>,
    input: u64,
    output: u64,
    created: i64,
}

struct MessageRow {
    id: String,
    session_id: String,
    role: String,
    parts: String,
    model: Option<String>,
    provider: Option<String>,
    created: i64,
    summary: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_parts_and_session_token_totals() {
        let directory = std::env::temp_dir().join(format!("aissence-crush-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("crush.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (id TEXT, parent_session_id TEXT, prompt_tokens INTEGER, completion_tokens INTEGER, created_at INTEGER);\
                 CREATE TABLE messages (id TEXT, session_id TEXT, role TEXT, parts TEXT, model TEXT, provider TEXT, created_at INTEGER, is_summary_message INTEGER);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sessions VALUES ('s1',NULL,12,4,1767225600000)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages VALUES ('u','s1','user',?1,NULL,NULL,1767225601000,0)",
                params![r#"[{"type":"text","data":{"text":"Change it"}}]"#],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO messages VALUES ('a','s1','assistant',?1,'gpt-5','openai',1767225602000,0)",
            params![r#"[{"type":"reasoning","data":{"thinking":"plan"}},{"type":"tool_call","data":{"id":"c1","name":"edit","input":"{\"path\":\"x\"}"}}]"#],
        ).unwrap();
        connection.execute(
            "INSERT INTO messages VALUES ('t','s1','tool',?1,NULL,NULL,1767225603000,0)",
            params![r#"[{"type":"tool_result","data":{"tool_call_id":"c1","content":"done","is_error":false}}]"#],
        ).unwrap();
        drop(connection);

        let sessions = load_database(&path, &directory).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message_count(), 3);
        assert_eq!(sessions[0].tool_uses().next().unwrap().1["path"], "x");
        assert_eq!(sessions[0].tool_results().next(), Some(("done", false)));
        assert_eq!(sessions[0].total_tokens().input, 12);
        let _ = std::fs::remove_dir_all(directory);
    }
}
