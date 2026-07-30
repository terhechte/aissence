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
    TimedMessage, TokenCount,
};
use crate::ncs;

/// Environment variable holding the OpenRouter API key.
pub const ENV_API_KEY: &str = "OPENROUTER_API_KEY";
/// Environment variable selecting the model (e.g. `inclusionai/ling-2.6-flash`).
pub const ENV_MODEL: &str = "OPENROUTER_MODEL";
/// Model used when [`ENV_MODEL`] is unset.
pub const DEFAULT_MODEL: &str = "inclusionai/ling-2.6-flash";

/// Long prose is summarized in bounded chunks so all of it contributes without
/// requiring one unbounded model context. Code-change renderings use one such
/// chunk because their complete synthesized diffs are persisted separately.
const SUMMARY_CHUNK_CHARS: usize = 6_000;
const CHANGE_SUMMARY_CHARS: usize = 6_000;

/// Summarizes commit groups using an OpenRouter-hosted model.
pub struct Summarizer {
    client: OpenRouterClient,
    model: String,
    /// Maximum concurrent stage-1 (per-item) requests.
    max_concurrency: usize,
    /// Items at most this many characters are used verbatim (no model call).
    passthrough_chars: usize,
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
        // Consecutive messages of the same kind (e.g. several "ai changed" in a
        // row) are merged so they become a single summary entry.
        let items = coalesce_consecutive(raw_items(group));
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
            .map(|(item, summary)| {
                let content = retained_content(&item);
                MessageSummary {
                    kind: item.kind,
                    summary,
                    content,
                    changes: item.changes,
                    timestamp: item.timestamp,
                    tokens: item.tokens,
                }
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

        // Finally, synthesize the three into one longer overview.
        let meta = self
            .meta_summary(&user_requested, &ai_answered, &ai_implemented, &commit_context)
            .await?;

        Ok(CommitSummary {
            user_requested,
            ai_answered,
            ai_implemented,
            messages,
            agents: crate::model::agents_for(group),
            meta,
            tokens: group.token_usage(),
        })
    }

    /// Synthesize the three component summaries into one cohesive overview (up
    /// to ~20 sentences): what the user wanted, what was implemented, and any
    /// important caveats the assistant noted.
    async fn meta_summary(
        &self,
        user_requested: &str,
        ai_answered: &str,
        ai_implemented: &str,
        commit_context: &str,
    ) -> Result<String, Error> {
        if user_requested.trim().is_empty()
            && ai_answered.trim().is_empty()
            && ai_implemented.trim().is_empty()
        {
            return Ok(String::new());
        }

        let system = "You write a single cohesive overview (up to 20 sentences) of the work \
             that led to a git commit, given three component summaries. Explain what the user \
             wanted, what the assistant implemented in the code, and anything important the \
             assistant noted (limitations, caveats, trade-offs, follow-ups). Write flowing \
             prose in past tense. Output only the overview — no preamble, headings, or bullets.";
        let user = format!(
            "Commit: {commit_context}\n\n\
             What the user requested:\n{user_requested}\n\n\
             What the assistant answered/explained:\n{ai_answered}\n\n\
             What the assistant implemented:\n{ai_implemented}\n\n\
             Write the overview (up to 20 sentences)."
        );
        self.complete(system, user, 1200).await
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
            jobs.push(async move { self.summarize_raw_item(kind, text).await });
        }
        let results: Vec<Result<String, Error>> = stream::iter(jobs)
            .buffered(self.max_concurrency)
            .collect()
            .await;
        results.into_iter().collect()
    }

    /// Summarize one source entry. Long user/assistant prose is split into
    /// bounded chunks, each chunk is summarized, and the partials are reduced
    /// into one entry summary. This lets the full source contribute without
    /// depending on an arbitrarily large model context window.
    async fn summarize_raw_item(&self, kind: MessageKind, text: String) -> Result<String, Error> {
        if text.chars().count() <= self.passthrough_chars {
            return Ok(text);
        }

        let category = Category::for_kind(kind);
        let input = summary_input(kind, &text);
        let chunks = chunk_chars(&input, SUMMARY_CHUNK_CHARS);
        let chunk_count = chunks.len();
        let mut partials = Vec::with_capacity(chunk_count);
        for (index, chunk) in chunks.into_iter().enumerate() {
            let labeled = if chunk_count == 1 {
                chunk
            } else {
                format!("Part {} of {chunk_count}:\n\n{chunk}", index + 1)
            };
            partials.push(
                self.complete(category.item_system(), category.item_user(&labeled), 300)
                    .await?,
            );
        }

        if partials.len() == 1 {
            return Ok(partials.pop().unwrap_or_default());
        }
        let joined = partials
            .iter()
            .enumerate()
            .map(|(index, summary)| format!("{}. {summary}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        self.complete(
            "You combine ordered partial summaries of one long message from a coding \
             session. Preserve all important requests, decisions, caveats, and conclusions \
             while removing repetition. Reply with one concise summary and no preamble.",
            format!(
                "Ordered partial summaries of one message:\n\n{joined}\n\n\
                 Write the combined summary."
            ),
            500,
        )
        .await
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
            500,
        )
        .await
    }

    /// Issue one chat completion and return the assistant's text.
    async fn complete(
        &self,
        system: &'static str,
        user: String,
        max_tokens: u32,
    ) -> Result<String, Error> {
        let request = ChatCompletionRequest::builder()
            .model(self.model.as_str())
            .messages(vec![
                Message::new(OrRole::System, system),
                Message::new(OrRole::User, user),
            ])
            .temperature(0.2)
            .max_tokens(max_tokens)
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
    /// Token usage attributed to this item (turn-level, approximate). A message
    /// that yields both an `AiWrote` and an `AiChanged` item puts its usage on
    /// the first, so the total across items still matches the group total.
    tokens: Option<TokenCount>,
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
                        tokens: message.token_usage,
                    });
                }
            }
            ncs::Role::Assistant => {
                // The message's usage covers its whole reply; attach it to the
                // first item it produces so it isn't counted twice.
                let mut tokens = message.token_usage;
                let text = message.text();
                if !text.trim().is_empty() {
                    items.push(RawItem {
                        kind: MessageKind::AiWrote,
                        text,
                        changes: Vec::new(),
                        timestamp: message.timestamp,
                        tokens: tokens.take(),
                    });
                }
                let changes = code_changes(message);
                if !changes.is_empty() {
                    items.push(RawItem {
                        kind: MessageKind::AiChanged,
                        text: render_changes(&changes),
                        changes,
                        timestamp: message.timestamp,
                        tokens: tokens.take(),
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

/// Merge runs of consecutive same-kind items into one. The merged item keeps
/// the first timestamp, joins the texts, and concatenates the code changes — so
/// e.g. five "ai changed" messages in a row become a single summarized entry.
fn coalesce_consecutive(items: Vec<RawItem>) -> Vec<RawItem> {
    let mut merged: Vec<RawItem> = Vec::with_capacity(items.len());
    for item in items {
        match merged.last_mut() {
            Some(last) if last.kind == item.kind => {
                if !last.text.is_empty() && !item.text.is_empty() {
                    last.text.push_str("\n\n");
                }
                last.text.push_str(&item.text);
                last.changes.extend(item.changes);
                if let Some(tokens) = item.tokens {
                    last.tokens.get_or_insert_with(TokenCount::default).add(&tokens);
                }
            }
            _ => merged.push(item),
        }
    }
    merged
}

/// Backfill token usage onto an already-produced summary from its group's
/// session data — without calling the model.
///
/// Token usage is a pure function of the parsed sessions, so this fills in the
/// per-commit total, per-agent/model totals, and per-message counts on summaries
/// created before token tracking existed (e.g. a cached `.aissence` entry). It
/// re-derives the same per-message item structure
/// [`Summarizer::summarize_group`] built the summary from, so the counts line up
/// with the existing `messages` by index; if that structure no longer matches
/// (the group changed), the exact commit and agent/model totals are still set.
pub fn attach_token_usage(group: &CommitGroup, summary: &mut CommitSummary) {
    summary.tokens = group.token_usage();
    summary.agents = crate::model::agents_for(group);
    let items = coalesce_consecutive(raw_items(group));
    if items_match_summary(&items, summary) {
        for (item, message) in items.iter().zip(summary.messages.iter_mut()) {
            message.tokens = item.tokens;
        }
    }
}

/// Backfill retained user/assistant source text onto a cached summary without
/// calling the model. Returns whether the summary changed.
///
/// Old `.aissence` files remain readable because the fields are optional. When
/// their message structure still matches the current source sessions, this
/// function upgrades them in place by attaching the complete user and assistant
/// text to each corresponding summary entry.
pub fn attach_source_content(group: &CommitGroup, summary: &mut CommitSummary) -> bool {
    let items = coalesce_consecutive(raw_items(group));
    if !items_match_summary(&items, summary) {
        return false;
    }

    let mut changed = false;
    for (item, message) in items.iter().zip(summary.messages.iter_mut()) {
        let content = retained_content(item);
        if message.content != content {
            message.content = content;
            changed = true;
        }
    }
    changed
}

fn items_match_summary(items: &[RawItem], summary: &CommitSummary) -> bool {
    items.len() == summary.messages.len()
        && items
            .iter()
            .zip(&summary.messages)
            .all(|(item, message)| item.kind == message.kind)
}

/// Source text persisted next to a message summary.
fn retained_content(item: &RawItem) -> Option<String> {
    match item.kind {
        MessageKind::UserWrote | MessageKind::AiWrote => Some(item.text.clone()),
        MessageKind::AiChanged => None,
    }
}

/// Text supplied to the stage-1 model. Preserve user and assistant prose in
/// full, and keep the existing bounded rendering for code changes (whose full
/// synthesized diffs are persisted separately).
fn summary_input(kind: MessageKind, text: &str) -> String {
    match kind {
        MessageKind::UserWrote | MessageKind::AiWrote => text.to_string(),
        MessageKind::AiChanged => truncate_chars(text, CHANGE_SUMMARY_CHARS),
    }
}

fn chunk_chars(text: &str, max_chars: usize) -> Vec<String> {
    assert!(max_chars > 0);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for character in text.chars() {
        current.push(character);
        count += 1;
        if count == max_chars {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }
    chunks
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
        .or_else(|| input.get("filePath"))
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

    let old = s("old_string")
        .or_else(|| s("oldString"))
        .or_else(|| s("old_str"))
        .or_else(|| s("oldText"));
    let new = s("new_string")
        .or_else(|| s("newString"))
        .or_else(|| s("new_str"))
        .or_else(|| s("newText"));
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

    if let Some(content) = s("content")
        .or_else(|| s("fileContent"))
        .or_else(|| s("contents"))
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: MessageKind, text: &str) -> RawItem {
        RawItem {
            kind,
            text: text.to_string(),
            changes: Vec::new(),
            timestamp: None,
            tokens: None,
        }
    }

    #[test]
    fn merges_runs_of_the_same_kind() {
        let items = vec![
            item(MessageKind::UserWrote, "a"),
            item(MessageKind::UserWrote, "b"),
            item(MessageKind::AiWrote, "c"),
            item(MessageKind::AiChanged, "d"),
            item(MessageKind::AiChanged, "e"),
            item(MessageKind::AiChanged, "f"),
            item(MessageKind::UserWrote, "g"),
        ];
        let merged = coalesce_consecutive(items);

        let kinds: Vec<_> = merged.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MessageKind::UserWrote,
                MessageKind::AiWrote,
                MessageKind::AiChanged,
                MessageKind::UserWrote,
            ]
        );
        assert_eq!(merged[0].text, "a\n\nb"); // two user messages merged
        assert_eq!(merged[2].text, "d\n\ne\n\nf"); // three "ai changed" merged
        assert_eq!(merged[3].text, "g"); // separated by the other kinds
    }

    #[test]
    fn retains_and_summarizes_user_and_assistant_text_without_a_length_cap() {
        let long_user = "u".repeat(30_000);
        let long_ai = "a".repeat(30_000);
        let user = item(MessageKind::UserWrote, &long_user);
        let ai = item(MessageKind::AiWrote, &long_ai);

        assert_eq!(retained_content(&user).as_deref(), Some(long_user.as_str()));
        assert_eq!(retained_content(&ai).as_deref(), Some(long_ai.as_str()));
        assert_eq!(summary_input(user.kind, &user.text), long_user);
        assert_eq!(summary_input(ai.kind, &ai.text), long_ai);
    }

    #[test]
    fn long_summary_input_is_chunked_without_losing_content() {
        let text = format!("{}🦀{}", "a".repeat(6_003), "b".repeat(6_004));
        let chunks = chunk_chars(&text, SUMMARY_CHUNK_CHARS);

        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 6_000));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn retained_content_preserves_coalesced_source_text() {
        let merged = coalesce_consecutive(vec![
            item(MessageKind::UserWrote, "first"),
            item(MessageKind::UserWrote, "second"),
        ]);

        assert_eq!(
            retained_content(&merged[0]).as_deref(),
            Some("first\n\nsecond")
        );
    }

    fn timed(role: ncs::Role, text: &str, tokens: Option<TokenCount>) -> TimedMessage {
        TimedMessage {
            timestamp: Some(chrono::Utc::now()),
            role,
            content: vec![ncs::ContentBlock::Text { text: text.into() }],
            session_id: None,
            provider: None,
            model: None,
            agent: None,
            session_path: std::path::PathBuf::from("test.jsonl"),
            token_usage: tokens,
        }
    }

    #[test]
    fn raw_items_attribute_message_usage_to_a_single_item() {
        let usage = TokenCount {
            input: 100,
            output: 40,
            cache_read: None,
            cache_create: None,
        };
        // An assistant message with both text and an edit: usage lands only on
        // the first item, so the per-item sum equals the message's usage.
        let mut assistant = timed(ncs::Role::Assistant, "here is the fix", Some(usage));
        assistant.content.push(ncs::ContentBlock::ToolUse {
            id: "1".into(),
            name: "Write".into(),
            input: serde_json::json!({ "file_path": "a.rs", "content": "x" }),
        });
        let group = CommitGroup {
            commit: None,
            messages: vec![timed(ncs::Role::User, "please fix", None), assistant],
            summary: None,
        };

        let items = raw_items(&group);
        assert_eq!(items.len(), 3); // user, ai wrote, ai changed
        assert_eq!(items[0].tokens, None); // user message
        assert_eq!(items[1].tokens, Some(usage)); // ai wrote carries it
        assert_eq!(items[2].tokens, None); // ai changed does not double count
    }

    #[test]
    fn attach_token_usage_fills_total_and_per_message() {
        let usage = TokenCount {
            input: 30,
            output: 12,
            cache_read: Some(4),
            cache_create: None,
        };
        let group = CommitGroup {
            commit: None,
            messages: vec![
                timed(ncs::Role::User, "hi", None),
                timed(ncs::Role::Assistant, "hello there", Some(usage)),
            ],
            summary: None,
        };
        // A summary whose messages line up with the group's items.
        let mut summary = CommitSummary {
            messages: vec![
                MessageSummary {
                    kind: MessageKind::UserWrote,
                    summary: "hi".into(),
                    content: None,
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
                MessageSummary {
                    kind: MessageKind::AiWrote,
                    summary: "hello".into(),
                    content: None,
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
            ],
            ..Default::default()
        };

        attach_token_usage(&group, &mut summary);
        let content_changed = attach_source_content(&group, &mut summary);

        assert_eq!(summary.tokens, Some(usage)); // exact conversation total
        assert_eq!(summary.agents.len(), 1);
        assert_eq!(summary.agents[0].tokens, Some(usage));
        assert_eq!(summary.messages[0].tokens, None); // user
        assert_eq!(summary.messages[1].tokens, Some(usage)); // assistant reply
        assert!(content_changed);
        assert_eq!(summary.messages[0].content.as_deref(), Some("hi"));
        assert_eq!(summary.messages[1].content.as_deref(), Some("hello there"));
        assert!(!attach_source_content(&group, &mut summary));
    }

    #[test]
    fn understands_opencode_edit_payload_names() {
        let input = serde_json::json!({
            "filePath": "src/lib.rs",
            "oldString": "old",
            "newString": "new"
        });
        let change = code_change("edit", &input);

        assert_eq!(change.file, "src/lib.rs");
        assert!(change.diff.contains("-old\n+new"));
    }
}
