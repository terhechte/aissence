//! LLM summarization of a commit's conversation and code changes, via
//! [OpenRouter](https://openrouter.ai/).
//!
//! For each [`CommitGroup`] we summarize every user entry, retain assistant
//! entries locally, and ask the model for the three commit-level summaries —
//! *user requested*, *ai answered*, and *ai implemented* — plus one cohesive
//! overview.
//!
//! The three categories are derived from the chat itself:
//! - **user**: [`ncs::Role::User`] message text,
//! - **responses**: [`ncs::Role::Assistant`] message text,
//! - **changes**: file-editing tool calls the assistant made (Edit/Write/…).
//!
//! ```no_run
//! # async fn run() -> Result<(), aissence::Error> {
//! let mut history = aissence::analyze("/path/to/project")?;
//! let summarizer = aissence::Summarizer::from_env()?;
//! // OPENROUTER_API_KEY / OPENROUTER_MODEL / OPENROUTER_PROVIDER
//! summarizer.summarize_history(&mut history).await?;
//! # Ok(()) }
//! ```

use std::sync::Arc;
use std::sync::Mutex;

use futures::stream::{self, StreamExt, TryStreamExt};
use openrouter_rs::OpenRouterClient;
use openrouter_rs::api::chat::{ChatCompletionRequest, Message};
use openrouter_rs::types::{ProviderPreferences, Role as OrRole};
use tokio::sync::Semaphore;

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
/// Environment variable selecting a preferred provider (e.g. `deepinfra`).
pub const ENV_PROVIDER: &str = "OPENROUTER_PROVIDER";
/// Model used when [`ENV_MODEL`] is unset.
pub const DEFAULT_MODEL: &str = "inclusionai/ling-2.6-flash";
/// Maximum number of commit groups summarized concurrently.
///
/// Each group can issue up to three concurrent aggregate requests before its
/// dependent `meta` request.
pub const MAX_CONCURRENT_GROUPS: usize = 8;
/// Maximum number of OpenRouter requests in flight across all commit groups.
pub const MAX_CONCURRENT_REQUESTS: usize = 24;

/// Actual billing reported by OpenRouter for one summarization operation.
///
/// Costs are denominated in US dollars. A request is counted as unpriced when
/// it succeeds but the provider response omits billing data.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SummarizationCost {
    pub usd: f64,
    pub request_count: usize,
    pub unpriced_request_count: usize,
}

impl SummarizationCost {
    pub fn add(&mut self, other: Self) {
        self.usd += other.usd;
        self.request_count += other.request_count;
        self.unpriced_request_count += other.unpriced_request_count;
    }
}

#[derive(Default)]
struct CostTracker(Mutex<SummarizationCost>);

impl CostTracker {
    fn record(&self, cost: Option<f64>) {
        let mut total = self.0.lock().expect("summarization cost lock");
        total.request_count += 1;
        match cost.filter(|value| value.is_finite() && *value >= 0.0) {
            Some(cost) => total.usd += cost,
            None => total.unpriced_request_count += 1,
        }
    }

    fn total(&self) -> SummarizationCost {
        *self.0.lock().expect("summarization cost lock")
    }
}

/// Summarizes commit groups using an OpenRouter-hosted model.
pub struct Summarizer {
    client: OpenRouterClient,
    model: String,
    provider: Option<String>,
    request_limit: Arc<Semaphore>,
}

impl Summarizer {
    /// Build a summarizer from the environment.
    ///
    /// Reads [`ENV_API_KEY`] (required), [`ENV_MODEL`] (optional, defaulting to
    /// [`DEFAULT_MODEL`]), and [`ENV_PROVIDER`] (optional).
    ///
    /// The provider is sent as the first entry in OpenRouter's provider
    /// preference order. OpenRouter may still fall back to another provider.
    pub fn from_env() -> Result<Self, Error> {
        let api_key = std::env::var(ENV_API_KEY).map_err(|_| Error::MissingEnv(ENV_API_KEY))?;
        let model = std::env::var(ENV_MODEL).unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let provider = std::env::var(ENV_PROVIDER)
            .ok()
            .map(|provider| provider.trim().to_string())
            .filter(|provider| !provider.is_empty());
        Self::new_with_provider(api_key, model, provider)
    }

    /// Build a summarizer with an explicit key and model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self, Error> {
        Self::new_with_provider(api_key, model, None)
    }

    /// Build a summarizer with an explicit key, model, and preferred provider.
    pub fn new_with_provider(
        api_key: impl Into<String>,
        model: impl Into<String>,
        provider: Option<String>,
    ) -> Result<Self, Error> {
        let client = OpenRouterClient::builder()
            .api_key(api_key.into())
            .build()
            .map_err(|e| Error::Llm(format!("client build: {e}")))?;
        Ok(Self {
            client,
            model: model.into(),
            provider,
            request_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        })
    }

    /// The model id this summarizer uses.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The preferred OpenRouter provider, if configured.
    pub fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    /// Summarize every non-empty group in `history`, filling in
    /// [`CommitGroup::summary`]. Commit groups use bounded concurrency, and the
    /// three independent category summaries within each group run concurrently.
    pub async fn summarize_history(&self, history: &mut CoalescedHistory) -> Result<(), Error> {
        let jobs = history
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| !group.messages.is_empty())
            .map(|(index, group)| async move {
                self.summarize_group(group)
                    .await
                    .map(|summary| (index, summary))
            });
        let summaries: Vec<(usize, CommitSummary)> = stream::iter(jobs)
            .buffer_unordered(MAX_CONCURRENT_GROUPS)
            .try_collect()
            .await?;

        for (index, summary) in summaries {
            history.groups[index].summary = Some(summary);
        }
        Ok(())
    }

    /// Summarize a single commit group. Every user entry gets a summary; the
    /// assistant-text and editing entries are retained without per-entry calls.
    pub async fn summarize_group(&self, group: &CommitGroup) -> Result<CommitSummary, Error> {
        self.summarize_group_with_cost(group).await.0
    }

    /// Summarize one group and return the actual OpenRouter cost incurred,
    /// including costs from requests made before a later request failed.
    pub async fn summarize_group_with_cost(
        &self,
        group: &CommitGroup,
    ) -> (Result<CommitSummary, Error>, SummarizationCost) {
        let costs = CostTracker::default();
        let outcome = self.summarize_group_tracked(group, &costs).await;
        (outcome, costs.total())
    }

    async fn summarize_group_tracked(
        &self,
        group: &CommitGroup,
        costs: &CostTracker,
    ) -> Result<CommitSummary, Error> {
        // Consecutive messages of the same kind (e.g. several "ai changed" in a
        // row) are merged locally into a single retained entry.
        let items = coalesce_consecutive(raw_items(group));
        let commit_context = group
            .commit
            .as_ref()
            .map(|c| c.summary.clone())
            .unwrap_or_else(|| "(uncommitted work in progress)".to_string());

        // Feed each category's complete retained source directly into its one
        // commit-level summary call.
        let by_kind = |kind: MessageKind| -> Vec<String> {
            items
                .iter()
                .filter(|item| item.kind == kind)
                .map(|item| item.text.clone())
                .collect()
        };
        let user = by_kind(MessageKind::UserWrote);
        let answered = by_kind(MessageKind::AiWrote);
        let changed = by_kind(MessageKind::AiChanged);

        // User-entry summaries and the three aggregate categories are
        // independent, so run all of them concurrently. The shared semaphore
        // bounds total request pressure across every concurrently active group.
        let (entry_summaries, user_requested, ai_answered, ai_implemented) = futures::try_join!(
            self.summarize_user_entries(&items, costs),
            self.reduce_items(&user, Category::User, &commit_context, costs),
            self.reduce_items(&answered, Category::Response, &commit_context, costs),
            self.reduce_items(&changed, Category::Change, &commit_context, costs),
        )?;

        let messages: Vec<MessageSummary> = items
            .iter()
            .zip(entry_summaries)
            .map(|(item, summary)| MessageSummary {
                kind: item.kind,
                summary,
                content: retained_content(item),
                changes: item.changes.clone(),
                timestamp: item.timestamp,
                tokens: item.tokens,
            })
            .collect();

        // Finally, synthesize the three into one longer overview.
        let meta = self
            .meta_summary(
                &user_requested,
                &ai_answered,
                &ai_implemented,
                &commit_context,
                costs,
            )
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

    /// Generate exactly one summary for every coalesced user entry. The full
    /// retained prompt is supplied with no character or entry-count cutoff.
    async fn summarize_user_entries(
        &self,
        items: &[RawItem],
        costs: &CostTracker,
    ) -> Result<Vec<Option<String>>, Error> {
        let mut jobs = Vec::with_capacity(items.len());
        for item in items {
            let kind = item.kind;
            let text = item.text.clone();
            jobs.push(async move {
                if kind != MessageKind::UserWrote {
                    return Ok(None);
                }
                self.complete(
                    "You summarize one complete user prompt from a coding session. \
                     Preserve every important request, constraint, question, correction, \
                     and reported problem. Reply with a concise 2-3 sentence summary and \
                     no preamble.",
                    format!("Complete user prompt:\n\n{text}"),
                    300,
                    costs,
                )
                .await
                .map(Some)
            });
        }
        stream::iter(jobs)
            .buffered(MAX_CONCURRENT_REQUESTS)
            .try_collect()
            .await
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
        costs: &CostTracker,
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
        self.complete(system, user, 1200, costs).await
    }

    /// Condense all source entries of one category into one commit-level
    /// summary. This is one model request regardless of entry count.
    async fn reduce_items(
        &self,
        source_items: &[String],
        category: Category,
        commit_context: &str,
        costs: &CostTracker,
    ) -> Result<String, Error> {
        let joined = source_items
            .iter()
            .filter(|s| !s.trim().is_empty())
            .enumerate()
            .map(|(i, s)| format!("{}. {}", i + 1, s.trim()))
            .collect::<Vec<_>>()
            .join("\n");

        if joined.is_empty() {
            return Ok(String::new());
        }

        self.complete(
            category.reduce_system(),
            category.reduce_user(&joined, commit_context),
            500,
            costs,
        )
        .await
    }

    /// Issue one chat completion and return the assistant's text.
    async fn complete(
        &self,
        system: &'static str,
        user: String,
        max_tokens: u32,
        costs: &CostTracker,
    ) -> Result<String, Error> {
        let _permit = self
            .request_limit
            .acquire()
            .await
            .map_err(|_| Error::Llm("summarization request limiter closed".to_string()))?;
        let mut request = ChatCompletionRequest::builder();
        request
            .model(self.model.as_str())
            .messages(vec![
                Message::new(OrRole::System, system),
                Message::new(OrRole::User, user),
            ])
            .temperature(0.2)
            .max_tokens(max_tokens);
        if let Some(provider) = &self.provider {
            let mut preferences = ProviderPreferences::default();
            preferences.order = Some(vec![provider.clone()]);
            request.provider(preferences);
        }
        let request = request
            .build()
            .map_err(|e| Error::Llm(format!("request build: {e}")))?;

        let response = self
            .client
            .chat()
            .create(&request)
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;
        costs.record(response.usage.as_ref().and_then(|usage| usage.cost));

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

/// One source message turned into a locally retained conversation entry and
/// aggregate-summary input.
struct RawItem {
    kind: MessageKind,
    /// Complete text used by the relevant commit-level summary call.
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
/// e.g. five "ai changed" messages in a row become a single retained entry.
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

/// Whether a cached summary satisfies the current per-entry contract.
///
/// Every coalesced user entry must have a non-empty summary. This deliberately
/// rejects files produced during the short-lived no-message-summary format so
/// "Summarize all" regenerates those commits, while accepting older caches that
/// already summarized user prompts.
pub fn has_required_user_summaries(group: &CommitGroup, summary: &CommitSummary) -> bool {
    let items = coalesce_consecutive(raw_items(group));
    items_match_summary(&items, summary)
        && summary
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::UserWrote)
            .all(|message| {
                message
                    .summary
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
            })
}

fn items_match_summary(items: &[RawItem], summary: &CommitSummary) -> bool {
    items.len() == summary.messages.len()
        && items
            .iter()
            .zip(&summary.messages)
            .all(|(item, message)| item.kind == message.kind)
}

/// Source text persisted with a conversation entry.
fn retained_content(item: &RawItem) -> Option<String> {
    match item.kind {
        MessageKind::UserWrote | MessageKind::AiWrote => Some(item.text.clone()),
        MessageKind::AiChanged => None,
    }
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

/// Distinguishes the three summary categories and their prompts.
#[derive(Clone, Copy)]
enum Category {
    User,
    Response,
    Change,
}

impl Category {
    /// System prompt for the commit-level aggregate step.
    fn reduce_system(self) -> &'static str {
        match self {
            Category::User => {
                "You write a concise 3-5 sentence summary of everything the user \
                 requested during the work that led to a git commit, given \
                 the complete ordered user messages. Write in past tense. Output only the \
                 summary, no preamble."
            }
            Category::Response => {
                "You write a concise 3-5 sentence summary of what the assistant \
                 answered and explained during the work that led to a git commit, \
                 given the complete ordered assistant replies. Preserve important \
                 decisions, conclusions, caveats, and trade-offs. Write in past tense. \
                 Output only the \
                 summary, no preamble."
            }
            Category::Change => {
                "You write a concise 3-5 sentence summary of what was implemented \
                 in the code for a git commit, given the complete ordered editing \
                 activity. Write in past tense, focusing on behavior and files touched. \
                 Output only the summary, no preamble."
            }
        }
    }

    /// User prompt for the commit-level aggregate step.
    fn reduce_user(self, joined: &str, commit_context: &str) -> String {
        let heading = match self {
            Category::User => "Complete ordered user messages",
            Category::Response => "Complete ordered assistant replies",
            Category::Change => "Complete ordered editing activity",
        };
        format!(
            "Commit: {commit_context}\n\n{heading}:\n{joined}\n\nWrite the 3-5 sentence summary."
        )
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
    fn retains_user_and_assistant_text_without_a_length_cap() {
        let long_user = "u".repeat(30_000);
        let long_ai = "a".repeat(30_000);
        let user = item(MessageKind::UserWrote, &long_user);
        let ai = item(MessageKind::AiWrote, &long_ai);

        assert_eq!(retained_content(&user).as_deref(), Some(long_user.as_str()));
        assert_eq!(retained_content(&ai).as_deref(), Some(long_ai.as_str()));
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
                    summary: Some("greeting".into()),
                    content: None,
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
                MessageSummary {
                    kind: MessageKind::AiWrote,
                    summary: None,
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
        assert!(has_required_user_summaries(&group, &summary));

        summary.messages[0].summary = None;
        assert!(!has_required_user_summaries(&group, &summary));
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

    #[test]
    fn cost_tracker_sums_reported_costs_and_counts_unpriced_requests() {
        let tracker = CostTracker::default();
        tracker.record(Some(0.001));
        tracker.record(None);
        tracker.record(Some(0.002));

        let total = tracker.total();
        assert!((total.usd - 0.003).abs() < f64::EPSILON);
        assert_eq!(total.request_count, 3);
        assert_eq!(total.unpriced_request_count, 1);
    }

    #[test]
    fn preferred_provider_uses_openrouter_order_format() {
        let mut preferences = ProviderPreferences::default();
        preferences.order = Some(vec!["deepinfra".to_string()]);

        assert_eq!(
            serde_json::to_value(preferences).unwrap(),
            serde_json::json!({ "order": ["deepinfra"] })
        );
    }

    #[test]
    fn constructor_retains_optional_provider() {
        let summarizer = Summarizer::new_with_provider(
            "test-key",
            "example/model",
            Some("deepinfra".to_string()),
        )
        .unwrap();

        assert_eq!(summarizer.model(), "example/model");
        assert_eq!(summarizer.provider(), Some("deepinfra"));
    }
}
