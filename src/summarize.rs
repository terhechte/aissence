//! LLM summarization of a commit's conversation and code changes, via
//! [OpenRouter](https://openrouter.ai/).
//!
//! For each [`CommitGroup`] we summarize every user entry and retain assistant
//! entries locally. Trivially short user entries are kept verbatim; the rest
//! are summarized in batched requests. The user-entry summaries are combined
//! locally into *user requested*. One additional, strictly bounded request —
//! issued concurrently with the user-entry summaries — derives *ai answered*
//! and *ai implemented* from bounded raw user evidence, terminal assistant
//! text, and an authoritative git fingerprint. A longer cohesive `meta`
//! overview remains optional.
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::stream::{self, StreamExt, TryStreamExt};
use openrouter_rs::OpenRouterClient;
use openrouter_rs::api::chat::{ChatCompletionRequest, Message, StreamOptions};
use openrouter_rs::error::OpenRouterError;
use openrouter_rs::types::completion::ResponseUsage;
use openrouter_rs::types::{ProviderPreferences, Role as OrRole};
use serde::Deserialize;
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
/// Groups are cheap to keep in flight; the global request limit
/// ([`MAX_CONCURRENT_REQUESTS`]) is the real governor of API pressure.
pub const MAX_CONCURRENT_GROUPS: usize = 32;
/// Maximum number of OpenRouter requests in flight across all commit groups.
pub const MAX_CONCURRENT_REQUESTS: usize = 24;
/// Number of retries after the initial request for transient failures.
/// Provider rate limits (429) can persist well beyond a few seconds when a
/// batch of groups starts at once, so the budget is generous.
const MAX_REQUEST_RETRIES: usize = 7;
/// Initial grace period before retrying; subsequent waits double.
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
/// Longest grace period between retries (before jitter).
const RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
/// Maximum persisted aggregate of the already-small user prompt summaries.
const USER_REQUESTED_CHARS: usize = 2_400;
/// User prompts at or below this length need no model call — the text itself
/// is already as concise as any summary the model would return.
const TRIVIAL_USER_CHARS: usize = 200;
/// Maximum user entries combined into one batched summary request.
const USER_BATCH_SIZE: usize = 8;
/// Maximum combined source characters in one batched summary request.
const USER_BATCH_CHARS: usize = 6_000;
/// Maximum user-request evidence sent to the compact core-summary call.
const CORE_USER_CHARS: usize = 1_600;
/// Maximum assistant evidence sent to the compact core-summary call.
const CORE_ASSISTANT_CHARS: usize = 3_600;
/// Maximum commit/edit fingerprint sent to the compact core-summary call.
const CORE_IMPLEMENTATION_CHARS: usize = 1_800;
/// Hard ceiling for the complete user message of the core-summary call.
const CORE_PROMPT_CHARS: usize = 8_000;
/// Maximum changed paths included in a committed implementation fingerprint.
const CORE_FILE_LIMIT: usize = 20;

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

/// The kind of model request currently being executed.
///
/// This is intentionally small and stable so front-ends can present request
/// progress without needing to know how prompts are assembled internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummarizationRequestKind {
    /// A request summarizing one or more user entries.
    UserBatch { entries: usize },
    /// The compact request that derives the answer and implementation fields.
    CoreOutcome,
    /// The optional long-form overview request.
    MetaSummary,
}

/// Token and billing usage reported when one request completes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SummarizationRequestUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cost: Option<f64>,
}

/// Live progress emitted by [`Summarizer::with_progress`].
///
/// While a request is streaming, [`Self::TokensReceived`] is an estimate based
/// on received text. The completed event contains the provider's exact usage
/// when OpenRouter returned it.
#[derive(Debug, Clone, PartialEq)]
pub enum SummarizationProgress {
    Started {
        id: usize,
        kind: SummarizationRequestKind,
    },
    TokensReceived {
        id: usize,
        tokens: usize,
    },
    Retrying {
        id: usize,
        kind: SummarizationRequestKind,
        attempt: usize,
        error: String,
    },
    Completed {
        id: usize,
        kind: SummarizationRequestKind,
        usage: Option<SummarizationRequestUsage>,
    },
    Failed {
        id: usize,
        kind: SummarizationRequestKind,
        error: String,
    },
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
    meta_summaries: bool,
    request_limit: Arc<Semaphore>,
    progress: Option<Arc<dyn Fn(SummarizationProgress) + Send + Sync>>,
    next_request_id: AtomicUsize,
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
            meta_summaries: false,
            request_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            progress: None,
            next_request_id: AtomicUsize::new(0),
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

    /// Enable or disable the optional long `meta` overview. The three core
    /// aggregate fields are generated regardless.
    pub fn with_meta_summaries(mut self, enabled: bool) -> Self {
        self.meta_summaries = enabled;
        self
    }

    /// Attach a callback that receives live request progress.
    ///
    /// Enabling progress also switches requests to OpenRouter's streaming
    /// endpoint so token estimates can be updated while a response is in
    /// flight. Callbacks should be quick; they run on the request task.
    pub fn with_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(SummarizationProgress) + Send + Sync + 'static,
    {
        self.progress = Some(Arc::new(callback));
        self
    }

    /// Whether the optional long `meta` overview is enabled.
    pub fn meta_summaries_enabled(&self) -> bool {
        self.meta_summaries
    }

    /// Summarize every non-empty group in `history`, filling in
    /// [`CommitGroup::summary`]. Commit groups use bounded concurrency; request
    /// pressure across user-entry and compact core-summary calls is globally
    /// limited.
    pub async fn summarize_history(&self, history: &mut CoalescedHistory) -> Result<(), Error> {
        self.summarize_history_with_cost(history).await.0
    }

    /// Summarize every non-empty group and return the actual OpenRouter cost.
    ///
    /// The returned cost includes successful requests made before a later
    /// request failed. As with [`Self::summarize_history`], summaries are only
    /// applied to `history` when the complete batch succeeds.
    pub async fn summarize_history_with_cost(
        &self,
        history: &mut CoalescedHistory,
    ) -> (Result<(), Error>, SummarizationCost) {
        let costs = CostTracker::default();
        let costs_ref = &costs;
        let jobs = history
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| !group.messages.is_empty())
            .map(|(index, group)| async move {
                self.summarize_group_tracked(group, costs_ref)
                    .await
                    .map(|summary| (index, summary))
            });
        let outcome: Result<Vec<(usize, CommitSummary)>, Error> = stream::iter(jobs)
            .buffer_unordered(MAX_CONCURRENT_GROUPS)
            .try_collect()
            .await;

        let summaries = match outcome {
            Ok(summaries) => summaries,
            Err(error) => return (Err(error), costs.total()),
        };

        for (index, summary) in summaries {
            history.groups[index].summary = Some(summary);
        }
        (Ok(()), costs.total())
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

        // The core-outcome call uses bounded raw user evidence, so it runs
        // concurrently with the user-entry summaries instead of waiting on
        // them — one model round trip less on the per-group critical path.
        let (entry_summaries, (ai_answered, ai_implemented)) = futures::try_join!(
            self.summarize_user_entries(&items, costs),
            self.summarize_core_outcome(&items, group.commit.as_ref(), costs),
        )?;
        let user_requested = aggregate_user_requests(&items, &entry_summaries);
        let meta = if self.meta_summaries {
            self.meta_summary(
                &user_requested,
                &ai_answered,
                &ai_implemented,
                &commit_context,
                costs,
            )
            .await?
        } else {
            String::new()
        };

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

    /// Generate both assistant-facing aggregate fields in one bounded request.
    ///
    /// The request never receives a full transcript or full diff. User evidence
    /// is bounded raw prompt text, assistant evidence favors the terminal
    /// response, and implementation evidence is an authoritative commit
    /// message/stat/path fingerprint. Pending work falls back to tool and path
    /// names, still without diff bodies.
    async fn summarize_core_outcome(
        &self,
        items: &[RawItem],
        commit: Option<&crate::model::CommitInfo>,
        costs: &CostTracker,
    ) -> Result<(String, String), Error> {
        let input = core_summary_input(items, commit);
        if !input.has_answer && !input.has_implementation {
            return Ok((String::new(), String::new()));
        }

        let fallback = fallback_core_outcome(items, commit);
        let text = self
            .complete(
                "You create two concise commit-level summaries from bounded evidence. \
                 Return one JSON object with exactly the string fields \"ai_answered\" and \
                 \"ai_implemented\". ai_answered states what the assistant concluded, \
                 explained, validated, or warned about; ignore intentions and progress \
                 updates. ai_implemented states only behavior actually supported by the \
                 committed implementation fingerprint; never treat a plan as completed \
                 work. Use 1-3 compact sentences per non-empty field. Use an empty string \
                 only when that category has no evidence. Output JSON only.",
                input.prompt,
                350,
                SummarizationRequestKind::CoreOutcome,
                costs,
            )
            .await?;
        let mut outcome = parse_core_outcome(&text).unwrap_or(fallback.clone());

        if !input.has_answer {
            outcome.ai_answered.clear();
        } else if outcome.ai_answered.trim().is_empty() {
            outcome.ai_answered = fallback.ai_answered;
        }
        if !input.has_implementation {
            outcome.ai_implemented.clear();
        } else if outcome.ai_implemented.trim().is_empty() {
            outcome.ai_implemented = fallback.ai_implemented;
        }

        Ok((
            outcome.ai_answered.trim().to_string(),
            outcome.ai_implemented.trim().to_string(),
        ))
    }

    /// Generate exactly one summary for every coalesced user entry. The full
    /// retained prompt is supplied with no character or entry-count cutoff.
    ///
    /// Trivially short entries are kept verbatim without a model call. The
    /// remaining entries are summarized in batched requests (one JSON array of
    /// summaries per batch) to cut round trips and request pressure; a batch
    /// whose response cannot be matched to its entries falls back to
    /// per-entry requests.
    async fn summarize_user_entries(
        &self,
        items: &[RawItem],
        costs: &CostTracker,
    ) -> Result<Vec<Option<String>>, Error> {
        let (mut results, batches) = plan_user_entry_jobs(items);
        let jobs = batches.into_iter().map(|batch| async move {
            let summaries = self.summarize_user_batch(items, &batch, costs).await?;
            Ok::<_, Error>((batch, summaries))
        });
        let outcomes: Vec<(Vec<usize>, Vec<String>)> = stream::iter(jobs)
            .buffered(MAX_CONCURRENT_REQUESTS)
            .try_collect()
            .await?;
        for (indices, summaries) in outcomes {
            for (index, summary) in indices.into_iter().zip(summaries) {
                results[index] = Some(summary);
            }
        }
        Ok(results)
    }

    /// Summarize one batch of user entries. Single-entry batches use the plain
    /// per-entry request; larger batches ask for one JSON array of summaries.
    async fn summarize_user_batch(
        &self,
        items: &[RawItem],
        batch: &[usize],
        costs: &CostTracker,
    ) -> Result<Vec<String>, Error> {
        if batch.len() == 1 {
            let summary = self
                .complete(
                    "You summarize one complete user prompt from a coding session. \
                     Preserve every important request, constraint, question, correction, \
                     and reported problem. Reply with a concise 2-3 sentence summary and \
                     no preamble.",
                    format!("Complete user prompt:\n\n{}", items[batch[0]].text),
                    300,
                    SummarizationRequestKind::UserBatch { entries: 1 },
                    costs,
                )
                .await?;
            return Ok(vec![summary]);
        }

        let mut prompt = String::from("Summarize each of these user prompts:\n\n");
        for (position, index) in batch.iter().enumerate() {
            prompt.push_str(&format!(
                "### User prompt {}\n{}\n\n",
                position + 1,
                items[*index].text
            ));
        }
        let max_tokens = (300 * batch.len() as u32).min(2_400);
        let text = self
            .complete(
                "You summarize several numbered user prompts from a coding session. \
                 For each, preserve every important request, constraint, question, \
                 correction, and reported problem. Return one JSON array of strings \
                 with exactly one concise 2-3 sentence summary per prompt, in the \
                 same order. Output JSON only.",
                prompt,
                max_tokens,
                SummarizationRequestKind::UserBatch {
                    entries: batch.len(),
                },
                costs,
            )
            .await?;

        let parsed = parse_string_array(&text).filter(|summaries| {
            summaries.len() == batch.len()
                && summaries.iter().all(|summary| !summary.trim().is_empty())
        });
        match parsed {
            Some(summaries) => Ok(summaries
                .into_iter()
                .map(|summary| summary.trim().to_string())
                .collect()),
            // The model returned something unusable — retry the batch's
            // entries individually.
            None => {
                let mut summaries = Vec::with_capacity(batch.len());
                for index in batch {
                    summaries.push(
                        self.complete(
                            "You summarize one complete user prompt from a coding session. \
                             Preserve every important request, constraint, question, correction, \
                             and reported problem. Reply with a concise 2-3 sentence summary and \
                             no preamble.",
                            format!("Complete user prompt:\n\n{}", items[*index].text),
                            300,
                            SummarizationRequestKind::UserBatch { entries: 1 },
                            costs,
                        )
                        .await?,
                    );
                }
                Ok(summaries)
            }
        }
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
        self.complete(
            system,
            user,
            1200,
            SummarizationRequestKind::MetaSummary,
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
        kind: SummarizationRequestKind,
        costs: &CostTracker,
    ) -> Result<String, Error> {
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
        if self.progress.is_some() {
            let mut stream_options = StreamOptions::default();
            stream_options.include_usage = Some(true);
            request.stream_options(stream_options);
        }
        let request = request
            .build()
            .map_err(|e| Error::Llm(format!("request build: {e}")))?;

        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed) + 1;
        let response = {
            let mut retry = 0;
            loop {
                let result = {
                    let _permit = self.request_limit.acquire().await.map_err(|_| {
                        Error::Llm("summarization request limiter closed".to_string())
                    })?;
                    if self.progress.is_some() {
                        self.emit_progress(SummarizationProgress::Started {
                            id: request_id,
                            kind,
                        });
                        self.stream_completion(&request, request_id).await
                    } else {
                        self.client.chat().create(&request).await.map(|response| {
                            CompletionResponse {
                                text: response
                                    .choices
                                    .first()
                                    .and_then(|choice| choice.content())
                                    .unwrap_or_default()
                                    .to_string(),
                                usage: response.usage,
                            }
                        })
                    }
                };
                match result {
                    Ok(response) => break response,
                    Err(error)
                        if is_retryable_request_error(&error) && retry < MAX_REQUEST_RETRIES =>
                    {
                        self.emit_progress(SummarizationProgress::Retrying {
                            id: request_id,
                            kind,
                            attempt: retry + 2,
                            error: error.to_string(),
                        });
                        tokio::time::sleep(retry_delay(retry)).await;
                        retry += 1;
                    }
                    Err(error) => {
                        let attempts = retry + 1;
                        let suffix = if attempts > 1 {
                            format!(" after {attempts} attempts")
                        } else {
                            String::new()
                        };
                        self.emit_progress(SummarizationProgress::Failed {
                            id: request_id,
                            kind,
                            error: format!("{error}{suffix}"),
                        });
                        return Err(Error::Llm(format!("{error}{suffix}")));
                    }
                }
            }
        };
        costs.record(response.usage.as_ref().and_then(|usage| usage.cost));
        self.emit_progress(SummarizationProgress::Completed {
            id: request_id,
            kind,
            usage: response.usage.as_ref().map(request_usage),
        });

        let text = response.text.trim().to_string();
        Ok(text)
    }

    async fn stream_completion(
        &self,
        request: &ChatCompletionRequest,
        request_id: usize,
    ) -> Result<CompletionResponse, OpenRouterError> {
        let mut stream = self.client.chat().stream(request).await?;
        let mut text = String::new();
        let mut received_chars = 0;
        let mut usage = None;

        while let Some(event) = stream.next().await {
            let response = event?;
            if let Some(response_usage) = response.usage {
                usage = Some(response_usage);
            }
            for choice in response.choices {
                if let Some(content) = choice.content() {
                    text.push_str(content);
                    received_chars += content.chars().count();
                }
                if let Some(reasoning) = choice.reasoning() {
                    received_chars += reasoning.chars().count();
                }
            }

            if received_chars > 0 {
                self.emit_progress(SummarizationProgress::TokensReceived {
                    id: request_id,
                    tokens: estimated_tokens(received_chars),
                });
            }
        }

        Ok(CompletionResponse { text, usage })
    }

    fn emit_progress(&self, progress: SummarizationProgress) {
        if let Some(callback) = &self.progress {
            callback(progress);
        }
    }
}

struct CompletionResponse {
    text: String,
    usage: Option<ResponseUsage>,
}

fn request_usage(usage: &ResponseUsage) -> SummarizationRequestUsage {
    SummarizationRequestUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cost: usage.cost,
    }
}

fn estimated_tokens(received_chars: usize) -> usize {
    received_chars.div_ceil(4)
}

struct CoreSummaryInput {
    prompt: String,
    has_answer: bool,
    has_implementation: bool,
}

#[derive(Clone, Default, Deserialize)]
struct CoreOutcome {
    #[serde(default)]
    ai_answered: String,
    #[serde(default)]
    ai_implemented: String,
}

/// Combine the summaries already generated for user entries without another
/// model request. Keep both the beginning and end when a long multi-turn
/// session exceeds the display budget, since later messages commonly refine
/// the original request.
fn aggregate_user_requests(items: &[RawItem], summaries: &[Option<String>]) -> String {
    let joined = items
        .iter()
        .zip(summaries)
        .filter_map(|(item, summary)| {
            (item.kind == MessageKind::UserWrote)
                .then_some(summary.as_deref())
                .flatten()
                .map(str::trim)
                .filter(|summary| !summary.is_empty())
        })
        .collect::<Vec<_>>()
        .join(" ");
    truncate_middle(&joined, USER_REQUESTED_CHARS)
}

/// Bounded raw user prompt text used as core-summary evidence. Unlike the
/// persisted aggregate this needs no model calls, so the core-outcome request
/// can start immediately, concurrently with the user-entry summaries.
fn user_evidence(items: &[RawItem]) -> String {
    items
        .iter()
        .filter(|item| item.kind == MessageKind::UserWrote)
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Partition user entries into summary jobs. Returns the prefilled results —
/// trivially short entries keep their text verbatim — plus the entry-index
/// batches that each need one model request. Entries at or above
/// [`USER_BATCH_CHARS`] always get a request of their own.
fn plan_user_entry_jobs(items: &[RawItem]) -> (Vec<Option<String>>, Vec<Vec<usize>>) {
    let mut prefilled: Vec<Option<String>> = (0..items.len()).map(|_| None).collect();
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_chars = 0_usize;
    for (index, item) in items.iter().enumerate() {
        if item.kind != MessageKind::UserWrote {
            continue;
        }
        let chars = item.text.chars().count();
        if chars <= TRIVIAL_USER_CHARS {
            prefilled[index] = Some(item.text.trim().to_string());
            continue;
        }
        if !current.is_empty()
            && (current.len() >= USER_BATCH_SIZE || current_chars + chars > USER_BATCH_CHARS)
        {
            batches.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current_chars += chars;
        current.push(index);
        if current_chars >= USER_BATCH_CHARS {
            batches.push(std::mem::take(&mut current));
            current_chars = 0;
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    (prefilled, batches)
}

/// Parse a JSON string array from a model response, tolerating surrounding
/// prose or code fences.
fn parse_string_array(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    let start = trimmed.find('[')?;
    let end = trimmed.rfind(']')?;
    (start <= end)
        .then(|| &trimmed[start..=end])
        .and_then(|json| serde_json::from_str(json).ok())
}

fn core_summary_input(
    items: &[RawItem],
    commit: Option<&crate::model::CommitInfo>,
) -> CoreSummaryInput {
    let user = user_evidence(items);
    let assistant = assistant_evidence(items);
    let implementation = implementation_fingerprint(items, commit);
    let has_answer = items
        .iter()
        .any(|item| item.kind == MessageKind::AiWrote && !item.text.trim().is_empty());
    let has_implementation = commit.is_some_and(|commit| commit.stats.files_changed > 0)
        || items
            .iter()
            .any(|item| item.kind == MessageKind::AiChanged && !item.changes.is_empty());

    let request = truncate_middle(&user, CORE_USER_CHARS);
    let assistant = truncate_head(&assistant, CORE_ASSISTANT_CHARS);
    let implementation = truncate_head(&implementation, CORE_IMPLEMENTATION_CHARS);
    let prompt = format!(
        "User requests (bounded raw excerpts):\n{}\n\n\
         Bounded assistant evidence, newest conclusion first:\n{}\n\n\
         Authoritative implementation fingerprint:\n{}\n\n\
         Produce the two-field JSON object.",
        nonempty_or_none(&request),
        nonempty_or_none(&assistant),
        nonempty_or_none(&implementation),
    );
    let prompt = truncate_head(&prompt, CORE_PROMPT_CHARS);

    CoreSummaryInput {
        prompt,
        has_answer,
        has_implementation,
    }
}

/// Favor the final assistant block, which coding agents normally use to recap
/// completed work. Add small tails of earlier blocks newest-first only while
/// the fixed evidence budget has room.
fn assistant_evidence(items: &[RawItem]) -> String {
    let replies = items
        .iter()
        .filter(|item| item.kind == MessageKind::AiWrote)
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    let Some(final_reply) = replies.last() else {
        return String::new();
    };

    let mut out = format!(
        "Most recent assistant text (tail):\n{}",
        truncate_tail(final_reply, 2_900)
    );
    for reply in replies.iter().rev().skip(1) {
        let remaining = CORE_ASSISTANT_CHARS.saturating_sub(out.chars().count());
        if remaining <= 80 {
            break;
        }
        let excerpt_chars = remaining.saturating_sub(38).min(420);
        out.push_str("\n\nEarlier conclusion candidate:\n");
        out.push_str(&truncate_tail(reply, excerpt_chars));
    }
    truncate_head(&out, CORE_ASSISTANT_CHARS)
}

/// Build compact implementation evidence without including code or diff
/// bodies. Committed work uses git's tree diff; only uncommitted work falls
/// back to recognized editing tools.
fn implementation_fingerprint(
    items: &[RawItem],
    commit: Option<&crate::model::CommitInfo>,
) -> String {
    if let Some(commit) = commit {
        if commit.stats.files_changed == 0 {
            return String::new();
        }

        let mut out = format!("Commit: {}", commit.summary.trim());
        if let Some(body) = commit
            .body
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push_str("\nCommit body: ");
            out.push_str(&truncate_head(body, 500));
        }
        out.push_str(&format!(
            "\nStats: {} files changed, +{} lines, -{} lines",
            commit.stats.files_changed, commit.stats.lines_added, commit.stats.lines_deleted
        ));
        if !commit.files.is_empty() {
            out.push_str("\nChanged paths:");
            for file in commit.files.iter().take(CORE_FILE_LIMIT) {
                out.push('\n');
                out.push_str(file.status.label());
                out.push(' ');
                if let Some(previous) = &file.previous_path {
                    out.push_str(previous);
                    out.push_str(" -> ");
                }
                out.push_str(&file.path);
            }
            if commit.files.len() > CORE_FILE_LIMIT {
                out.push_str(&format!(
                    "\n… and {} more changed paths",
                    commit.files.len() - CORE_FILE_LIMIT
                ));
            }
        }
        return truncate_head(&out, CORE_IMPLEMENTATION_CHARS);
    }

    let changes = items
        .iter()
        .filter(|item| item.kind == MessageKind::AiChanged)
        .flat_map(|item| &item.changes)
        .collect::<Vec<_>>();
    if changes.is_empty() {
        return String::new();
    }

    let mut out = format!("Uncommitted editing actions: {}", changes.len());
    let mut seen = std::collections::HashSet::new();
    for change in changes {
        if change.file == "(unknown file)" || !seen.insert(change.file.as_str()) {
            continue;
        }
        out.push('\n');
        out.push_str(&change.tool);
        out.push(' ');
        out.push_str(&change.file);
        if seen.len() == CORE_FILE_LIMIT {
            break;
        }
    }
    truncate_head(&out, CORE_IMPLEMENTATION_CHARS)
}

fn fallback_core_outcome(
    items: &[RawItem],
    commit: Option<&crate::model::CommitInfo>,
) -> CoreOutcome {
    let ai_answered = items
        .iter()
        .rev()
        .find(|item| item.kind == MessageKind::AiWrote && !item.text.trim().is_empty())
        .map(|item| normalize_whitespace(&truncate_tail(item.text.trim(), 900)))
        .unwrap_or_default();

    let ai_implemented = commit
        .filter(|commit| commit.stats.files_changed > 0)
        .map(|commit| {
            format!(
                "{} Changed {} files (+{}/-{} lines).",
                commit.summary.trim(),
                commit.stats.files_changed,
                commit.stats.lines_added,
                commit.stats.lines_deleted
            )
        })
        .unwrap_or_else(|| {
            let fingerprint = implementation_fingerprint(items, None);
            normalize_whitespace(&truncate_head(&fingerprint, 900))
        });

    CoreOutcome {
        ai_answered,
        ai_implemented,
    }
}

fn parse_core_outcome(text: &str) -> Option<CoreOutcome> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start <= end)
        .then(|| &trimmed[start..=end])
        .and_then(|json| serde_json::from_str(json).ok())
}

fn nonempty_or_none(text: &str) -> &str {
    if text.trim().is_empty() {
        "(none)"
    } else {
        text
    }
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_head(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", text.chars().take(keep).collect::<String>())
}

fn truncate_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let tail = text
        .chars()
        .skip(count.saturating_sub(keep))
        .collect::<String>();
    format!("…{tail}")
}

fn truncate_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".chars().take(max_chars).collect();
    }
    let available = max_chars - 1;
    let head_chars = available / 2;
    let tail_chars = available - head_chars;
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .skip(count.saturating_sub(tail_chars))
        .collect::<String>();
    format!("{head}…{tail}")
}

fn is_retryable_request_error(error: &OpenRouterError) -> bool {
    match error {
        OpenRouterError::Api(context) => is_retryable_status(context.status.as_u16()),
        OpenRouterError::HttpRequest(_) => true,
        _ => false,
    }
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// Exponential backoff with a cap and up to +50% jitter, so the many
/// in-flight requests don't all retry in lockstep after a shared 429.
fn retry_delay(retry: usize) -> Duration {
    let base = RETRY_BASE_DELAY
        .saturating_mul(1_u32 << retry.min(10))
        .min(RETRY_MAX_DELAY);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::from(elapsed.subsec_nanos()))
        .unwrap_or(0);
    let jitter = Duration::from_millis(nanos % (base.as_millis() as u64 / 2 + 1));
    base + jitter
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
                    last.tokens
                        .get_or_insert_with(TokenCount::default)
                        .add(&tokens);
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

/// Fill missing core aggregate fields on a cached summary without calling the
/// model. Existing non-empty values are never replaced.
///
/// This gives older caches a useful migration path: user request summaries are
/// combined locally, the terminal assistant response supplies an extractive
/// answer fallback, and committed git facts supply the implementation fallback.
pub fn attach_core_fallbacks(group: &CommitGroup, summary: &mut CommitSummary) -> bool {
    let items = coalesce_consecutive(raw_items(group));
    if !items_match_summary(&items, summary) {
        return false;
    }

    let mut changed = false;
    if summary.user_requested.trim().is_empty() {
        let entry_summaries = summary
            .messages
            .iter()
            .map(|message| message.summary.clone())
            .collect::<Vec<_>>();
        let aggregate = aggregate_user_requests(&items, &entry_summaries);
        if !aggregate.is_empty() {
            summary.user_requested = aggregate;
            changed = true;
        }
    }

    let fallback = fallback_core_outcome(&items, group.commit.as_ref());
    if summary.ai_answered.trim().is_empty() && !fallback.ai_answered.is_empty() {
        summary.ai_answered = fallback.ai_answered;
        changed = true;
    }
    if summary.ai_implemented.trim().is_empty() && !fallback.ai_implemented.is_empty() {
        summary.ai_implemented = fallback.ai_implemented;
        changed = true;
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

/// Whether every source category needed by the UI has a core aggregate.
pub fn has_required_core_summaries(group: &CommitGroup, summary: &CommitSummary) -> bool {
    let items = coalesce_consecutive(raw_items(group));
    let has_user = items.iter().any(|item| item.kind == MessageKind::UserWrote);
    let has_answer = items.iter().any(|item| item.kind == MessageKind::AiWrote);
    let has_implementation = group
        .commit
        .as_ref()
        .is_some_and(|commit| commit.stats.files_changed > 0)
        || items.iter().any(|item| item.kind == MessageKind::AiChanged);
    let present = |text: &str| !text.trim().is_empty();

    (!has_user || present(&summary.user_requested))
        && (!has_answer || present(&summary.ai_answered))
        && (!has_implementation || present(&summary.ai_implemented))
}

/// Whether an enabled optional `meta` overview is present.
pub fn has_required_meta_summary(summary: &CommitSummary) -> bool {
    let has_core = [
        &summary.user_requested,
        &summary.ai_answered,
        &summary.ai_implemented,
    ]
    .iter()
    .any(|text| !text.trim().is_empty());
    !has_core || !summary.meta.trim().is_empty()
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
            let old = edit
                .get("old_string")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let new = edit
                .get("new_string")
                .and_then(|v| v.as_str())
                .unwrap_or("");
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
    fn cached_core_fallbacks_fill_only_missing_fields() {
        let group = CommitGroup {
            commit: Some(crate::model::CommitInfo {
                id: "abc".into(),
                short_id: "abc".into(),
                summary: "Fix cached summaries".into(),
                body: None,
                message: "Fix cached summaries".into(),
                author_name: "tester".into(),
                author_email: "t@example.com".into(),
                time: chrono::Utc::now(),
                stats: crate::model::CommitStats {
                    files_changed: 1,
                    lines_added: 5,
                    lines_deleted: 2,
                },
                files: vec![],
            }),
            messages: vec![
                timed(ncs::Role::User, "please fix it", None),
                timed(ncs::Role::Assistant, "Fixed it and ran tests.", None),
            ],
            summary: None,
        };
        let mut summary = CommitSummary {
            messages: vec![
                MessageSummary {
                    kind: MessageKind::UserWrote,
                    summary: Some("The user requested a fix.".into()),
                    content: Some("please fix it".into()),
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
                MessageSummary {
                    kind: MessageKind::AiWrote,
                    summary: None,
                    content: Some("Fixed it and ran tests.".into()),
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
            ],
            ..Default::default()
        };

        assert!(attach_core_fallbacks(&group, &mut summary));
        assert_eq!(summary.user_requested, "The user requested a fix.");
        assert_eq!(summary.ai_answered, "Fixed it and ran tests.");
        assert!(summary.ai_implemented.contains("Fix cached summaries"));
        assert!(!attach_core_fallbacks(&group, &mut summary));

        summary.ai_answered = "Keep this richer cached answer.".into();
        assert!(!attach_core_fallbacks(&group, &mut summary));
        assert_eq!(summary.ai_answered, "Keep this richer cached answer.");
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

    #[tokio::test]
    async fn empty_history_returns_zero_cost_without_a_request() {
        let summarizer = Summarizer::new("test-key", "test/model").unwrap();
        let mut history = CoalescedHistory {
            folder: std::path::PathBuf::from("/test/project"),
            groups: vec![],
            undated: vec![],
            warnings: vec![],
        };

        let (outcome, cost) = summarizer.summarize_history_with_cost(&mut history).await;

        assert!(outcome.is_ok());
        assert_eq!(cost, SummarizationCost::default());
    }

    #[test]
    fn transient_failures_use_exponential_retry_delays() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        // Base delays double from 2s, with up to +50% jitter.
        for (retry, expected_secs) in [(0, 2), (1, 4), (2, 8)] {
            let base = Duration::from_secs(expected_secs);
            let delay = retry_delay(retry);
            assert!(delay >= base, "retry {retry}: {delay:?} below base");
            assert!(
                delay <= base + base / 2 + Duration::from_millis(1),
                "retry {retry}: {delay:?} above jitter bound"
            );
        }
        // High retries are capped at RETRY_MAX_DELAY (plus jitter).
        let high = retry_delay(10);
        assert!(high >= RETRY_MAX_DELAY);
        assert!(high <= RETRY_MAX_DELAY + RETRY_MAX_DELAY / 2 + Duration::from_millis(1));
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
        assert!(!summarizer.meta_summaries_enabled());

        let summarizer = summarizer.with_meta_summaries(true);
        assert!(summarizer.meta_summaries_enabled());
    }

    #[test]
    fn core_and_optional_meta_requirements_are_independent() {
        let group = CommitGroup {
            commit: None,
            messages: vec![
                timed(ncs::Role::User, "please fix it", None),
                timed(ncs::Role::Assistant, "I fixed it", None),
            ],
            summary: None,
        };
        let summary = CommitSummary {
            user_requested: "Asked for a fix.".into(),
            ai_answered: "Confirmed the fix.".into(),
            ai_implemented: String::new(),
            meta: "The user requested a fix and the assistant completed it.".into(),
            ..Default::default()
        };

        assert!(has_required_core_summaries(&group, &summary));
        assert!(has_required_meta_summary(&summary));

        let without_meta = CommitSummary {
            meta: String::new(),
            ..summary.clone()
        };
        assert!(has_required_core_summaries(&group, &without_meta));
        assert!(!has_required_meta_summary(&without_meta));
    }

    #[test]
    fn user_request_aggregate_is_local_bounded_and_keeps_both_ends() {
        let items = vec![
            item(MessageKind::UserWrote, "first"),
            item(MessageKind::AiWrote, "reply"),
            item(MessageKind::UserWrote, "last"),
        ];
        let summaries = vec![
            Some(format!("FIRST {}", "a".repeat(2_000))),
            None,
            Some(format!("{} LAST", "b".repeat(2_000))),
        ];

        let aggregate = aggregate_user_requests(&items, &summaries);
        assert!(aggregate.chars().count() <= USER_REQUESTED_CHARS);
        assert!(aggregate.starts_with("FIRST"));
        assert!(aggregate.ends_with("LAST"));
    }

    #[test]
    fn compact_core_prompt_has_a_hard_bound_and_excludes_diff_bodies() {
        let huge_reply = format!(
            "old progress SECRET_MIDDLE {}\nFinal result: tests pass.",
            "x".repeat(20_000)
        );
        let huge_diff = format!(
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n{}DO_NOT_SEND_DIFF_BODY",
            "+code\n".repeat(20_000)
        );
        let mut changed = item(MessageKind::AiChanged, &huge_diff);
        changed.changes.push(CodeChange {
            tool: "apply_patch".into(),
            file: "src/lib.rs".into(),
            diff: huge_diff,
        });
        let items = vec![
            item(MessageKind::UserWrote, "fix it"),
            item(MessageKind::AiWrote, &huge_reply),
            changed,
        ];
        let commit = crate::model::CommitInfo {
            id: "abc".into(),
            short_id: "abc".into(),
            summary: "Fix compact summaries".into(),
            body: Some("Use bounded evidence.".into()),
            message: "Fix compact summaries\n\nUse bounded evidence.".into(),
            author_name: "tester".into(),
            author_email: "t@example.com".into(),
            time: chrono::Utc::now(),
            stats: crate::model::CommitStats {
                files_changed: 1,
                lines_added: 10,
                lines_deleted: 3,
            },
            files: vec![crate::model::CommitFileChange {
                status: crate::model::CommitFileStatus::Modified,
                path: "src/lib.rs".into(),
                previous_path: None,
            }],
        };

        let input = core_summary_input(&items, Some(&commit));
        assert!(input.prompt.chars().count() <= CORE_PROMPT_CHARS);
        assert!(input.prompt.contains("fix it"));
        assert!(input.prompt.contains("Final result: tests pass."));
        assert!(input.prompt.contains("M src/lib.rs"));
        assert!(!input.prompt.contains("DO_NOT_SEND_DIFF_BODY"));
        assert!(!input.prompt.contains("SECRET_MIDDLE"));
        assert!(input.has_answer);
        assert!(input.has_implementation);
    }

    #[test]
    fn parses_json_even_when_a_model_wraps_it_in_a_code_fence() {
        let parsed = parse_core_outcome(
            "```json\n{\"ai_answered\":\"Done.\",\"ai_implemented\":\"Changed it.\"}\n```",
        )
        .unwrap();
        assert_eq!(parsed.ai_answered, "Done.");
        assert_eq!(parsed.ai_implemented, "Changed it.");
    }

    #[test]
    fn parses_string_arrays_even_inside_code_fences() {
        let parsed = parse_string_array("```json\n[\"one\",\"two\"]\n```").unwrap();
        assert_eq!(parsed, vec!["one".to_string(), "two".to_string()]);
        assert!(parse_string_array("no json here").is_none());
    }

    #[test]
    fn user_entry_jobs_keep_trivial_prompts_verbatim_and_batch_the_rest() {
        let items = vec![
            item(MessageKind::UserWrote, "ok"),  // trivial: no model call
            item(MessageKind::AiWrote, "reply"), // not a user entry
            item(MessageKind::UserWrote, &"a".repeat(500)),
            item(MessageKind::UserWrote, &"b".repeat(500)),
            item(MessageKind::UserWrote, &"c".repeat(7_000)), // own request
        ];

        let (prefilled, batches) = plan_user_entry_jobs(&items);

        assert_eq!(prefilled[0].as_deref(), Some("ok"));
        assert_eq!(prefilled[1], None);
        assert_eq!(batches, vec![vec![2, 3], vec![4]]);
    }

    #[test]
    fn user_entry_jobs_cap_batch_size() {
        let items = (0..10)
            .map(|_| item(MessageKind::UserWrote, &"a".repeat(500)))
            .collect::<Vec<_>>();

        let (prefilled, batches) = plan_user_entry_jobs(&items);

        assert!(prefilled.iter().all(|summary| summary.is_none()));
        assert_eq!(
            batches,
            vec![vec![0, 1, 2, 3, 4, 5, 6, 7], vec![8, 9]],
            "batches stop at USER_BATCH_SIZE entries"
        );
    }

    #[test]
    fn user_evidence_joins_only_user_text() {
        let items = vec![
            item(MessageKind::UserWrote, "first"),
            item(MessageKind::AiWrote, "reply"),
            item(MessageKind::UserWrote, "last"),
        ];

        assert_eq!(user_evidence(&items), "first\n\nlast");
    }

    #[test]
    fn fallback_core_outcome_uses_terminal_answer_and_commit_facts() {
        let items = vec![
            item(MessageKind::AiWrote, "I will investigate."),
            item(MessageKind::AiWrote, "Implemented the fix. All tests pass."),
        ];
        let commit = crate::model::CommitInfo {
            id: "abc".into(),
            short_id: "abc".into(),
            summary: "Fix the bug".into(),
            body: None,
            message: "Fix the bug".into(),
            author_name: "tester".into(),
            author_email: "t@example.com".into(),
            time: chrono::Utc::now(),
            stats: crate::model::CommitStats {
                files_changed: 2,
                lines_added: 12,
                lines_deleted: 4,
            },
            files: vec![],
        };

        let fallback = fallback_core_outcome(&items, Some(&commit));
        assert_eq!(fallback.ai_answered, "Implemented the fix. All tests pass.");
        assert!(fallback.ai_implemented.contains("Fix the bug"));
        assert!(fallback.ai_implemented.contains("2 files"));
    }
}
