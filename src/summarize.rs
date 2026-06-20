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
use crate::model::{CoalescedHistory, CommitGroup, CommitSummary, TimedMessage};
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

    /// Summarize a single commit group into its three-part [`CommitSummary`].
    pub async fn summarize_group(&self, group: &CommitGroup) -> Result<CommitSummary, Error> {
        let items = GroupItems::from_group(group);
        let commit_context = group
            .commit
            .as_ref()
            .map(|c| c.summary.clone())
            .unwrap_or_else(|| "(uncommitted work in progress)".to_string());

        // The three categories are independent — run them concurrently.
        let (user_requested, ai_answered, ai_implemented) = futures::try_join!(
            self.summarize_category(&items.user, Category::User, &commit_context),
            self.summarize_category(&items.responses, Category::Response, &commit_context),
            self.summarize_category(&items.changes, Category::Change, &commit_context),
        )?;

        Ok(CommitSummary {
            user_requested,
            ai_answered,
            ai_implemented,
        })
    }

    /// Map-reduce one category of items into a single 3-5 sentence summary.
    async fn summarize_category(
        &self,
        items: &[String],
        category: Category,
        commit_context: &str,
    ) -> Result<String, Error> {
        if items.is_empty() {
            return Ok(String::new());
        }
        let per_item = self.map_items(items, category).await?;
        self.reduce_items(&per_item, category, commit_context).await
    }

    /// Stage 1: summarize each item, concurrently and order-preserving. Items
    /// shorter than [`Self::passthrough_chars`] are returned verbatim.
    async fn map_items(&self, items: &[String], category: Category) -> Result<Vec<String>, Error> {
        let results: Vec<Result<String, Error>> = stream::iter(items.iter().map(|item| async move {
            if item.chars().count() <= self.passthrough_chars {
                return Ok(item.clone());
            }
            let truncated = truncate_chars(item, self.item_char_budget);
            self.complete(category.item_system(), category.item_user(&truncated))
                .await
        }))
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

/// The items of a commit group, bucketed by category.
struct GroupItems {
    user: Vec<String>,
    responses: Vec<String>,
    changes: Vec<String>,
}

impl GroupItems {
    fn from_group(group: &CommitGroup) -> Self {
        let mut user = Vec::new();
        let mut responses = Vec::new();
        let mut changes = Vec::new();

        for message in &group.messages {
            match message.role {
                ncs::Role::User => push_nonempty(&mut user, message.text()),
                ncs::Role::Assistant => {
                    push_nonempty(&mut responses, message.text());
                    changes.extend(change_items(message));
                }
                // Tool results and system notices aren't user intent, the
                // assistant's answer, or a code change — skip them.
                ncs::Role::Tool | ncs::Role::System => {}
            }
        }

        Self {
            user,
            responses,
            changes,
        }
    }
}

fn push_nonempty(into: &mut Vec<String>, value: String) {
    if !value.trim().is_empty() {
        into.push(value);
    }
}

/// Render each file-editing tool call in `message` as a text item.
fn change_items(message: &TimedMessage) -> Vec<String> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ncs::ContentBlock::ToolUse { name, input, .. } if is_edit_tool(name) => {
                let file = input
                    .get("file_path")
                    .or_else(|| input.get("path"))
                    .or_else(|| input.get("filename"))
                    .or_else(|| input.get("file"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(unknown file)");
                Some(format!("Tool `{name}` on `{file}`:\n{}", input))
            }
            _ => None,
        })
        .collect()
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
