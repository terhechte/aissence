//! LLM summarization of a commit's conversation and code changes, via
//! [OpenRouter](https://openrouter.ai/).
//!
//! For each [`CommitGroup`] we produce a [`CommitSummary`] with three parts —
//! *user requested*, *ai answered*, *ai implemented* — using a two-stage
//! map-reduce:
//!
//! 1. **Map**: every individual item (each user message, each assistant reply,
//!    each file-editing tool call) is summarized into 2-3 sentences. Short items
//!    are passed through verbatim to avoid a needless model call.
//! 2. **Reduce**: the per-item summaries of each category are condensed into one
//!    slightly longer (3-5 sentence) summary.
//!
//! The three categories are derived from the chat itself:
//! - **user**: [`ncs::Role::User`] message text,
//! - **responses**: [`ncs::Role::Assistant`] message text,
//! - **changes**: file-editing tool calls the assistant made (Edit/Write/…).
//!
//! ```no_run
//! # async fn run() -> Result<(), aissence::Error> {
//! let mut history = aissence::analyze("/path/to/project")?;
//! let summarizer = aissence::Summarizer::from_env()?; // OPENROUTER_API_KEY / OPENROUTER_MODEL
//! summarizer.summarize_history(&mut history).await?;
//! # Ok(()) }
//! ```

use futures::stream::{self, StreamExt};
use openrouter_rs::api::chat::{ChatCompletionRequest, Message};
use openrouter_rs::types::Role as OrRole;
use openrouter_rs::OpenRouterClient;

use crate::error::Error;
use crate::model::{
    CoalescedHistory, CodeChange, CommitGroup, CommitSummary, MessageKind, MessageSummary,
    TimedMessage,
};
use crate::ncs;

/// Environment variable holding the OpenRouter API key.
pub const ENV_API_KEY: &str = "OPENROUTER_API_KEY";
/// Environment variable selecting the model (e.g. `inclusionai/ling-2.6-flash`).
pub const ENV_MODEL: &str = "OPENROUTER_MODEL";
/// Model used when [`ENV_MODEL`] is unset.
pub const DEFAULT_MODEL: &str = "inclusionai/ling-2.6-flash";

/// Summarizes commit groups using an OpenRouter-hosted model.
pub struct Summarizer {
    client: OpenRouterClient,
    model: String,
    /// Maximum concurrent stage-1 (per-item) requests.
    max_concurrency: usize,
    /// Items at most this many characters are used verbatim (no model call).
    passthrough_chars: usize,
    /// Items longer than this are truncated before being sent to the model.
    item_char_budget: usize,
}

impl Summarizer {
    /// Build a summarizer from the environment.
    ///
    /// Reads [`ENV_API_KEY`] (required) and [`ENV_MODEL`] (optional, defaulting
    /// to [`DEFAULT_MODEL`]).
    pub fn from_env() -> Result<Self, Error> {
        let api_key = std::env::var(ENV_API_KEY).map_err(|_| Error::MissingEnv(ENV_API_KEY))?;
        let model = std::env::var(ENV_MODEL).unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        Self::new(api_key, model)
    }

    /// Build a summarizer with an explicit key and model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self, Error> {
        let client = OpenRouterClient::builder()
            .api_key(api_key.into())
            .build()
            .map_err(|e| Error::Llm(format!("client build: {e}")))?;
        Ok(Self {
            client,
            model: model.into(),
            max_concurrency: 6,
            passthrough_chars: 320,
            item_char_budget: 6_000,
        })
    }

    /// The model id this summarizer uses.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Summarize every non-empty group in `history`, filling in
    /// [`CommitGroup::summary`]. Groups are processed sequentially (each group
    /// already parallelizes its own per-item requests internally).
    pub async fn summarize_history(&self, history: &mut CoalescedHistory) -> Result<(), Error> {
        for group in &mut history.groups {
            if group.messages.is_empty() {
                continue;
            }
            let summary = self.summarize_group(group).await?;
            group.summary = Some(summary);
        }
        Ok(())
    }

    /// Summarize a single commit group: a per-message breakdown plus the three
    /// aggregated overviews.
    pub async fn summarize_group(&self, group: &CommitGroup) -> Result<CommitSummary, Error> {
        let items = raw_items(group);
        let commit_context = group
            .commit
            .as_ref()
            .map(|c| c.summary.clone())
            .unwrap_or_else(|| "(uncommitted work in progress)".to_string());

        // Stage 1 (map): summarize every message, concurrently.
        let per_item = self.map_raw_items(&items).await?;

        // Per-message breakdown (drives the app's middle pane).
        let messages: Vec<MessageSummary> = items
            .into_iter()
            .zip(per_item)
            .map(|(item, summary)| MessageSummary {
                kind: item.kind,
                summary,
                changes: item.changes,
                timestamp: item.timestamp,
            })
            .collect();

        // Stage 2 (reduce): aggregate each category from its message summaries.
        let by_kind = |kind: MessageKind| -> Vec<String> {
            messages
                .iter()
                .filter(|m| m.kind == kind)
                .map(|m| m.summary.clone())
                .collect()
        };
        let user = by_kind(MessageKind::UserWrote);
        let answered = by_kind(MessageKind::AiWrote);
        let changed = by_kind(MessageKind::AiChanged);

        // The three categories are independent — run them concurrently.
        let (user_requested, ai_answered, ai_implemented) = futures::try_join!(
            self.reduce_items(&user, Category::User, &commit_context),
            self.reduce_items(&answered, Category::Response, &commit_context),
            self.reduce_items(&changed, Category::Change, &commit_context),
        )?;

        Ok(CommitSummary {
            user_requested,
            ai_answered,
            ai_implemented,
            messages,
        })
    }

    /// Stage 1: summarize each message, concurrently and order-preserving.
    /// Messages shorter than [`Self::passthrough_chars`] are used verbatim.
    async fn map_raw_items(&self, items: &[RawItem]) -> Result<Vec<String>, Error> {
        // Build the futures in a plain loop (not `.map`) so each one captures
        // owned data and a uniform `&self` borrow — avoids the higher-ranked
        // closure-lifetime bound that breaks `tokio::spawn`'s `Send + 'static`.
        let mut jobs = Vec::with_capacity(items.len());
        for item in items {
            let kind = item.kind;
            let text = item.text.clone();
            jobs.push(async move {
                if text.chars().count() <= self.passthrough_chars {
                    return Ok::<String, Error>(text);
                }
                let category = Category::for_kind(kind);
                let truncated = truncate_chars(&text, self.item_char_budget);
                self.complete(category.item_system(), category.item_user(&truncated))
                    .await
            });
        }
        let results: Vec<Result<String, Error>> = stream::iter(jobs)
            .buffered(self.max_concurrency)
            .collect()
            .await;
        results.into_iter().collect()
    }

    /// Stage 2: condense per-item summaries into one final summary.
    async fn reduce_items(
        &self,
        per_item: &[String],
        category: Category,
        commit_context: &str,
    ) -> Result<String, Error> {
        let joined = per_item
            .iter()
            .filter(|s| !s.trim().is_empty())
            .enumerate()
            .map(|(i, s)| format!("{}. {}", i + 1, s.trim()))
            .collect::<Vec<_>>()
            .join("\n");

        if joined.is_empty() {
            return Ok(String::new());
        }
        // A single item needs no further reduction.
        if per_item.len() == 1 {
            return Ok(per_item[0].trim().to_string());
        }

        self.complete(
            category.reduce_system(),
            category.reduce_user(&joined, commit_context),
        )
        .await
    }

    /// Issue one chat completion and return the assistant's text.
    async fn complete(&self, system: &'static str, user: String) -> Result<String, Error> {
        let request = ChatCompletionRequest::builder()
            .model(self.model.as_str())
            .messages(vec![
                Message::new(OrRole::System, system),
                Message::new(OrRole::User, user),
            ])
            .temperature(0.2)
            .max_tokens(500)
            .build()
            .map_err(|e| Error::Llm(format!("request build: {e}")))?;

        let response = self
            .client
            .chat()
            .create(&request)
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let text = response
            .choices
            .first()
            .and_then(|choice| choice.content())
            .unwrap_or_default()
            .trim()
            .to_string();
        Ok(text)
    }
}

/// One message turned into a unit of summarization.
struct RawItem {
    kind: MessageKind,
    /// Text fed to the model (message text, or a rendering of the edits).
    text: String,
    /// Code changes — only for [`MessageKind::AiChanged`].
    changes: Vec<CodeChange>,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// Split a commit group into ordered per-message [`RawItem`]s. A single
/// assistant message can yield both an `AiWrote` (its text) and an `AiChanged`
/// (its edits) item.
fn raw_items(group: &CommitGroup) -> Vec<RawItem> {
    let mut items = Vec::new();
    for message in &group.messages {
        match message.role {
            ncs::Role::User => {
                let text = message.text();
                if !text.trim().is_empty() {
                    items.push(RawItem {
                        kind: MessageKind::UserWrote,
                        text,
                        changes: Vec::new(),
                        timestamp: message.timestamp,
                    });
                }
            }
            ncs::Role::Assistant => {
                let text = message.text();
                if !text.trim().is_empty() {
                    items.push(RawItem {
                        kind: MessageKind::AiWrote,
                        text,
                        changes: Vec::new(),
                        timestamp: message.timestamp,
                    });
                }
                let changes = code_changes(message);
                if !changes.is_empty() {
                    items.push(RawItem {
                        kind: MessageKind::AiChanged,
                        text: render_changes(&changes),
                        changes,
                        timestamp: message.timestamp,
                    });
                }
            }
            // Tool results and system notices aren't user intent, the
            // assistant's answer, or a code change — skip them.
            ncs::Role::Tool | ncs::Role::System => {}
        }
    }
    items
}

/// Extract the file-editing tool calls of `message` as [`CodeChange`]s.
fn code_changes(message: &TimedMessage) -> Vec<CodeChange> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ncs::ContentBlock::ToolUse { name, input, .. } if is_edit_tool(name) => {
                Some(code_change(name, input))
            }
            _ => None,
        })
        .collect()
}

/// Build a [`CodeChange`] (with a synthesized unified diff) from one tool call.
fn code_change(name: &str, input: &serde_json::Value) -> CodeChange {
    let file = input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("filename"))
        .or_else(|| input.get("file"))
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown file)")
        .to_string();
    let diff = synth_diff(&file, input);
    CodeChange {
        tool: name.to_string(),
        file,
        diff,
    }
}

/// Render code changes as text for the model to summarize.
fn render_changes(changes: &[CodeChange]) -> String {
    changes
        .iter()
        .map(|c| format!("{} `{}`:\n{}", c.tool, c.file, c.diff))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Best-effort unified diff from a tool-call input. Handles `old_string`/
/// `new_string` (Edit/replace), an `edits` array (MultiEdit), and `content`
/// (Write — shown as a new file); otherwise dumps any stringy payload as
/// additions.
fn synth_diff(file: &str, input: &serde_json::Value) -> String {
    let s = |key: &str| input.get(key).and_then(|v| v.as_str());

    let old = s("old_string").or_else(|| s("old_str")).or_else(|| s("oldText"));
    let new = s("new_string").or_else(|| s("new_str")).or_else(|| s("newText"));
    if let (Some(old), Some(new)) = (old, new) {
        return diff_old_new(file, old, new);
    }

    if let Some(edits) = input.get("edits").and_then(|v| v.as_array()) {
        let mut out = format!("--- a/{file}\n+++ b/{file}\n");
        for edit in edits {
            let old = edit.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let new = edit.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str("@@ @@\n");
            push_prefixed(&mut out, '-', old);
            push_prefixed(&mut out, '+', new);
        }
        return out;
    }

    if let Some(content) = s("content").or_else(|| s("contents")) {
        return added_file(file, content);
    }

    let dump = s("patch")
        .or_else(|| s("input"))
        .map(str::to_string)
        .unwrap_or_else(|| input.to_string());
    added_file(file, &dump)
}

fn diff_old_new(file: &str, old: &str, new: &str) -> String {
    let oc = old.lines().count().max(1);
    let nc = new.lines().count().max(1);
    let mut out = format!("--- a/{file}\n+++ b/{file}\n@@ -1,{oc} +1,{nc} @@\n");
    push_prefixed(&mut out, '-', old);
    push_prefixed(&mut out, '+', new);
    out
}

fn added_file(file: &str, content: &str) -> String {
    let nc = content.lines().count().max(1);
    let mut out = format!("--- /dev/null\n+++ b/{file}\n@@ -0,0 +1,{nc} @@\n");
    push_prefixed(&mut out, '+', content);
    out
}

/// Append each line of `text`, prefixed by the diff `marker`.
fn push_prefixed(out: &mut String, marker: char, text: &str) {
    for line in text.lines() {
        out.push(marker);
        out.push_str(line);
        out.push('\n');
    }
}

/// Whether a tool name denotes a code/file modification across the supported
/// agents (Claude Code, Codex, Gemini).
fn is_edit_tool(name: &str) -> bool {
    const EDIT_TOOLS: &[&str] = &[
        "edit",
        "write",
        "multiedit",
        "notebookedit",
        "write_file",
        "edit_file",
        "replace",
        "apply_patch",
        "create_file",
        "str_replace",
        "str_replace_editor",
        "str_replace_based_edit_tool",
    ];
    let lower = name.to_ascii_lowercase();
    EDIT_TOOLS.contains(&lower.as_str())
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\n…[truncated]")
}

/// Distinguishes the three summary categories and their prompts.
#[derive(Clone, Copy)]
enum Category {
    User,
    Response,
    Change,
}

impl Category {
    /// Map a per-message kind to its prompt category.
    fn for_kind(kind: MessageKind) -> Self {
        match kind {
            MessageKind::UserWrote => Category::User,
            MessageKind::AiWrote => Category::Response,
            MessageKind::AiChanged => Category::Change,
        }
    }

    /// System prompt for the stage-1 per-item summary.
    fn item_system(self) -> &'static str {
        match self {
            Category::User => {
                "You summarize a single user message from a coding session. \
                 Reply with 2-3 sentences describing what the user asked for or \
                 reported. Output only the summary, no preamble."
            }
            Category::Response => {
                "You summarize a single assistant message from a coding session. \
                 Reply with 2-3 sentences describing what the assistant explained, \
                 decided, or concluded. Output only the summary, no preamble."
            }
            Category::Change => {
                "You summarize a single code-change action (a file-editing tool \
                 call) from a coding session. Reply with 2-3 sentences describing \
                 what was changed and in which file. Output only the summary, no \
                 preamble."
            }
        }
    }

    /// User prompt wrapping one item for stage 1.
    fn item_user(self, item: &str) -> String {
        let label = match self {
            Category::User => "User message",
            Category::Response => "Assistant message",
            Category::Change => "Code change",
        };
        format!("{label}:\n\n{item}")
    }

    /// System prompt for the stage-2 reduce step.
    fn reduce_system(self) -> &'static str {
        match self {
            Category::User => {
                "You write a concise 3-5 sentence summary of everything the user \
                 requested during the work that led to a git commit, given \
                 per-message summaries. Write in past tense. Output only the \
                 summary, no preamble."
            }
            Category::Response => {
                "You write a concise 3-5 sentence summary of what the assistant \
                 answered and explained during the work that led to a git commit, \
                 given per-message summaries. Write in past tense. Output only the \
                 summary, no preamble."
            }
            Category::Change => {
                "You write a concise 3-5 sentence summary of what was implemented \
                 in the code for a git commit, given per-change summaries. Write \
                 in past tense, focusing on behavior and files touched. Output \
                 only the summary, no preamble."
            }
        }
    }

    /// User prompt for the stage-2 reduce step.
    fn reduce_user(self, joined: &str, commit_context: &str) -> String {
        let heading = match self {
            Category::User => "Per-message summaries of what the user requested",
            Category::Response => "Per-message summaries of what the assistant answered",
            Category::Change => "Per-change summaries of what was implemented",
        };
        format!("Commit: {commit_context}\n\n{heading}:\n{joined}\n\nWrite the 3-5 sentence summary.")
    }
}
