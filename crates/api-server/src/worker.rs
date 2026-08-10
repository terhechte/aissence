use std::time::Duration;

use aissence::{AissenceCommit, CommitSummary, SummarizationCost};
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

pub const HOSTED_REQUEST_RETRIES: usize = 1;
const HOSTED_CORE_INPUT_CHARS: usize = 8_192;
const HOSTED_FIXED_INPUT_CHARS_PER_REQUEST: usize = 4_096;
const HOSTED_PUBLIC_PROMPT_CHARS: usize = 32_000;

struct Job {
    id: Uuid,
    import_id: Uuid,
    commit_id: Uuid,
    uploader_id: Uuid,
    kind: String,
    commit: AissenceCommit,
    attempt_id: Option<Uuid>,
}

pub async fn run(state: AppState) {
    loop {
        match claim(&state).await {
            Ok(Some(job)) => process(&state, job).await,
            Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(error) => {
                tracing::error!(error = %error, "summarization worker failed to claim a job");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn claim(state: &AppState) -> Result<Option<Job>, Box<dyn std::error::Error + Send + Sync>> {
    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;

    transaction
        .execute(
            "update public.summary_jobs set \
               status=case when attempts < 3 then 'queued'::public.summary_job_status \
                           else 'failed'::public.summary_job_status end, \
               error_detail=case when attempts < 3 then 'recovered stale worker lease' \
                                 else 'summary worker repeatedly lost its lease' end, \
               lease_expires_at=null, completed_at=case when attempts < 3 then null else now() end \
             where status='processing' and lease_expires_at < now()",
            &[],
        )
        .await?;

    let row = transaction
        .query_opt(
            "with candidate as ( \
                select sj.id from public.summary_jobs sj \
                where sj.status='queued' order by sj.created_at \
                for update skip locked limit 1 \
             ) \
             update public.summary_jobs sj set status='processing', attempts=attempts+1, \
                 started_at=now(), lease_expires_at=now()+interval '10 minutes' \
             from candidate c, public.imports i \
             where sj.id=c.id and i.id=sj.import_id \
             returning sj.id,sj.import_id,sj.commit_id,i.uploader_id,sj.kind::text,sj.payload",
            &[],
        )
        .await?;
    if let Some(row) = &row
        && row.get::<_, String>(4) == "public"
    {
        transaction
            .execute(
                "update public.commits set summary_status='processing' where id=$1",
                &[&row.get::<_, Uuid>(2)],
            )
            .await?;
    }
    transaction.commit().await?;
    let Some(row) = row else { return Ok(None) };
    let payload: Value = row.get(5);
    Ok(Some(Job {
        id: row.get(0),
        import_id: row.get(1),
        commit_id: row.get(2),
        uploader_id: row.get(3),
        kind: row.get(4),
        commit: serde_json::from_value(payload)?,
        attempt_id: None,
    }))
}

async fn process(state: &AppState, mut job: Job) {
    let public_prompt_input_chars = job
        .commit
        .source
        .messages
        .iter()
        .filter(|message| message.kind == aissence::MessageKind::UserWrote)
        .filter_map(|message| message.content.as_deref())
        .filter(|content| !content.trim().is_empty())
        .map(str::chars)
        .map(|chars| chars.count().min(6_000))
        .fold(0usize, usize::saturating_add);
    let subscriber_prompt_input_chars = job
        .commit
        .source
        .messages
        .iter()
        .filter(|message| message.kind == aissence::MessageKind::UserWrote)
        .filter_map(|message| message.content.as_deref())
        .filter(|content| content.chars().count() > 200)
        .map(str::chars)
        .map(|chars| chars.count().min(6_000))
        .fold(0usize, usize::saturating_add);
    let (request_ceiling, input_char_ceiling) = if job.kind == "public" {
        let planned = planned_public_requests(&job.commit);
        // Public generation summarizes every prompt, allows a worst-case
        // individual retry for each failed batch member, then synthesizes the
        // story from those summaries.
        hosted_usage_ceiling(
            planned,
            HOSTED_PUBLIC_PROMPT_CHARS.saturating_add(public_prompt_input_chars.saturating_mul(2)),
            state.summarizer.fallback_model().is_some(),
        )
    } else {
        let planned = planned_subscriber_requests(&job.commit);
        // Reserve for the initial batches plus worst-case individual fallback,
        // which can send the bounded prompt evidence a second time.
        hosted_usage_ceiling(
            planned,
            subscriber_prompt_input_chars.saturating_mul(2),
            state.summarizer.fallback_model().is_some(),
        )
    };
    match reserve_budget(state, &job, request_ceiling, input_char_ceiling).await {
        Ok(Some(attempt_id)) => job.attempt_id = Some(attempt_id),
        Ok(None) => {
            fail_job(
                state,
                &job,
                "daily summarization safety limit reached",
                None,
            )
            .await;
            return;
        }
        Err(error) => {
            fail_job(
                state,
                &job,
                &format!("usage reservation failed: {error}"),
                None,
            )
            .await;
            return;
        }
    }

    if job.kind == "public" {
        let (result, cost) = state
            .summarizer
            .summarize_document_commit_with_cost(&job.commit)
            .await;
        match result {
            Ok(summary) => {
                let mut summary = summary;
                aissence::Redactor::global()
                    .redact_summary_with_stats(&mut summary, job.commit.stats);
                job.commit.summary = Some(summary.clone());
                if let Err(error) = succeed_public_job(state, &job, &summary, cost).await {
                    tracing::error!(error = %error, job_id = %job.id, "cannot persist public story");
                }
            }
            Err(error) => fail_job(state, &job, &error.to_string(), Some(cost)).await,
        }
    } else {
        let (result, cost) = state
            .summarizer
            .summarize_subscriber_prompts_with_cost(&job.commit)
            .await;
        match result {
            Ok(outcome) => {
                if let Err(error) = finish_subscriber_job(state, &job, &outcome, cost).await {
                    tracing::error!(error = %error, job_id = %job.id, "cannot persist subscriber summaries");
                }
            }
            Err(error) => fail_job(state, &job, &error.to_string(), Some(cost)).await,
        }
    }
}

fn hosted_usage_ceiling(
    planned_requests: usize,
    source_input_chars: usize,
    has_fallback_model: bool,
) -> (usize, usize) {
    if planned_requests == 0 {
        return (0, 0);
    }
    let model_attempts = if has_fallback_model { 2 } else { 1 };
    let request_ceiling = planned_requests
        .saturating_mul(HOSTED_REQUEST_RETRIES + 1)
        .saturating_mul(model_attempts);
    let input_char_ceiling = source_input_chars
        .saturating_add(HOSTED_CORE_INPUT_CHARS)
        .saturating_mul(HOSTED_REQUEST_RETRIES + 1)
        .saturating_mul(model_attempts)
        .saturating_add(request_ceiling.saturating_mul(HOSTED_FIXED_INPUT_CHARS_PER_REQUEST));
    (request_ceiling, input_char_ceiling)
}

fn planned_public_requests(commit: &AissenceCommit) -> usize {
    let mut batch_requests = 0usize;
    let mut individual_fallbacks = 0usize;
    let mut batch_entries = 0usize;
    let mut batch_chars = 0usize;
    for message in commit
        .source
        .messages
        .iter()
        .filter(|message| message.kind == aissence::MessageKind::UserWrote)
    {
        let Some(content) = message
            .content
            .as_deref()
            .map(str::trim)
            .filter(|content| !content.is_empty())
        else {
            continue;
        };
        let chars = content.chars().count().min(6_000);
        individual_fallbacks += 1;
        if batch_entries > 0 && (batch_entries >= 8 || batch_chars + chars > 6_000) {
            batch_requests += 1;
            batch_entries = 0;
            batch_chars = 0;
        }
        batch_entries += 1;
        batch_chars += chars;
        if batch_chars >= 6_000 {
            batch_requests += 1;
            batch_entries = 0;
            batch_chars = 0;
        }
    }
    // One final public-story synthesis request follows the prompt-summary
    // batches. Empty-prompt commits are not queued by the database policy.
    1 + batch_requests + usize::from(batch_entries > 0) + individual_fallbacks
}

fn planned_subscriber_requests(commit: &AissenceCommit) -> usize {
    let mut requests = 0usize;
    let mut individual_fallbacks = 0usize;
    let mut batch_entries = 0usize;
    let mut batch_chars = 0usize;
    for message in commit
        .source
        .messages
        .iter()
        .filter(|message| message.kind == aissence::MessageKind::UserWrote)
    {
        let chars = message
            .content
            .as_deref()
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or(0)
            .min(6_000);
        if chars <= 200 {
            continue;
        }
        individual_fallbacks += 1;
        if batch_entries > 0 && (batch_entries >= 8 || batch_chars + chars > 6_000) {
            requests += 1;
            batch_entries = 0;
            batch_chars = 0;
        }
        batch_entries += 1;
        batch_chars += chars;
        if batch_chars >= 6_000 {
            requests += 1;
            batch_entries = 0;
            batch_chars = 0;
        }
    }
    requests + usize::from(batch_entries > 0) + individual_fallbacks
}

async fn reserve_budget(
    state: &AppState,
    job: &Job,
    request_ceiling: usize,
    input_char_ceiling: usize,
) -> Result<Option<Uuid>, Box<dyn std::error::Error + Send + Sync>> {
    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;
    transaction
        .query_one("select pg_advisory_xact_lock(764294831)", &[])
        .await?;
    transaction
        .query_one(
            "select pg_advisory_xact_lock(hashtextextended($1::text,0))",
            &[&job.uploader_id],
        )
        .await?;
    let user_usage = transaction
        .query_one(
            "select coalesce(sum(reserved_request_count),0)::bigint, \
             coalesce(sum(reserved_input_chars),0)::bigint from public.summarization_attempts \
             where uploader_id=$1 and created_at >= date_trunc('day',now())",
            &[&job.uploader_id],
        )
        .await?;
    let global_usage = transaction
        .query_one(
            "select coalesce(sum(reserved_request_count),0)::bigint, \
             coalesce(sum(reserved_input_chars),0)::bigint from public.summarization_attempts \
             where created_at >= date_trunc('day',now())",
            &[],
        )
        .await?;
    let user_requests: i64 = user_usage.get(0);
    let user_chars: i64 = user_usage.get(1);
    let global_requests: i64 = global_usage.get(0);
    let global_chars: i64 = global_usage.get(1);
    let request_ceiling_i32 = i32::try_from(request_ceiling)?;
    let request_ceiling = i64::from(request_ceiling_i32);
    let input_char_ceiling = i64::try_from(input_char_ceiling)?;
    if user_requests.saturating_add(request_ceiling) > state.config.daily_user_request_limit
        || global_requests.saturating_add(request_ceiling) > state.config.daily_global_request_limit
        || user_chars.saturating_add(input_char_ceiling) > state.config.daily_user_input_chars
        || global_chars.saturating_add(input_char_ceiling) > state.config.daily_global_input_chars
    {
        transaction.commit().await?;
        return Ok(None);
    }
    let attempt_id: Uuid = transaction
        .query_one(
            "insert into public.summarization_attempts \
             (import_id,commit_id,summary_job_id,uploader_id,request_count,reserved_request_count,reserved_input_chars,status) \
             values ($1,$2,$3,$4,0,$5,$6,'reserved') returning id",
            &[
                &job.import_id,
                &job.commit_id,
                &job.id,
                &job.uploader_id,
                &request_ceiling_i32,
                &input_char_ceiling,
            ],
        )
        .await?
        .get(0);
    transaction.commit().await?;
    Ok(Some(attempt_id))
}

async fn succeed_public_job(
    state: &AppState,
    job: &Job,
    summary: &CommitSummary,
    cost: SummarizationCost,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;
    let tokens = summary.tokens;
    let token_input = checked_token(tokens.map(|tokens| tokens.input))?;
    let token_output = checked_token(tokens.map(|tokens| tokens.output))?;
    let token_cache_read = checked_token(tokens.and_then(|tokens| tokens.cache_read))?;
    let token_cache_create = checked_token(tokens.and_then(|tokens| tokens.cache_create))?;
    transaction
        .execute(
            "update public.commits set public_story=$2,public_subline=$3,public_beats=$4, \
             token_input=$7::bigint,token_output=$8::bigint, \
             token_cache_read=$9::bigint,token_cache_create=$10::bigint, \
             summary_origin=$5,summarizer_version=$6,summary_status='succeeded',summarized_at=now(),summary_error=null \
             where id=$1",
            &[
                &job.commit_id,
                &summary.public.summary,
                &"",
                &serde_json::to_value(&summary.public.beats)?,
                &summary_origin(summary.public.origin),
                &(aissence::SUMMARIZER_VERSION as i32),
                &token_input,
                &token_output,
                &token_cache_read,
                &token_cache_create,
            ],
        )
        .await?;
    let payload = serde_json::to_value(&job.commit)?;
    transaction
        .execute(
            "update public.import_commits set payload=$3 where import_id=$1 and commit_id=$2",
            &[&job.import_id, &job.commit_id, &payload],
        )
        .await?;
    transaction
        .execute(
            "update public.summary_jobs set status='succeeded',error_detail=null, \
             lease_expires_at=null,completed_at=now() where id=$1",
            &[&job.id],
        )
        .await?;
    record_attempt(&transaction, job, "succeeded", None, cost).await?;
    transaction.commit().await?;
    Ok(())
}

async fn finish_subscriber_job(
    state: &AppState,
    job: &Job,
    outcome: &aissence::PromptSummarizationOutcome,
    cost: SummarizationCost,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut retry_commit = job.commit.clone();
    let retry_stats = retry_commit.stats;
    let summary = retry_commit
        .summary
        .get_or_insert_with(aissence::CommitSummary::default);
    if summary.messages.len() != retry_commit.source.messages.len() {
        summary.messages = retry_commit.source.messages.clone();
    }
    for (message_index, generated) in &outcome.summaries {
        if let Some(message) = summary.messages.get_mut(*message_index) {
            message.subscriber_summary = Some(generated.clone());
        }
    }
    summary.prompt_failures = outcome.failures.clone();
    aissence::Redactor::global().redact_summary_with_stats(summary, retry_stats);
    let retry_payload = serde_json::to_value(&retry_commit)?;

    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;
    for (sort_order, summary) in &outcome.summaries {
        let sort_order = i32::try_from(*sort_order)?;
        transaction
            .execute(
                "insert into public.message_summaries (message_id,summary,summarizer_version) \
                 select id,$3,$4 from public.messages \
                 where commit_id=$1 and sort_order=$2 and kind='UserWrote' \
                 on conflict (message_id) do update set summary=excluded.summary, \
                 summarizer_version=excluded.summarizer_version, generated_at=now()",
                &[
                    &job.commit_id,
                    &sort_order,
                    summary,
                    &(aissence::SUMMARIZER_VERSION as i32),
                ],
            )
            .await?;
    }
    let failure_detail = (!outcome.failures.is_empty()).then(|| {
        let first = &outcome.failures[0];
        format!(
            "{} prompt summary failure(s) remain; first unresolved message {}: {}",
            outcome.failures.len(),
            first.message_index,
            first.error
        )
    });
    let status = if failure_detail.is_some() {
        "failed"
    } else {
        "succeeded"
    };
    transaction
        .execute(
            "update public.summary_jobs set status=$2::public.summary_job_status,error_detail=$3, \
             payload=$4,lease_expires_at=null,completed_at=now() where id=$1",
            &[&job.id, &status, &failure_detail, &retry_payload],
        )
        .await?;
    record_attempt(&transaction, job, status, failure_detail.as_deref(), cost).await?;
    transaction.commit().await?;
    Ok(())
}

async fn fail_job(state: &AppState, job: &Job, detail: &str, cost: Option<SummarizationCost>) {
    let outcome: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
        let mut client = state.pool.get().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                "update public.summary_jobs set status='failed',error_detail=$2, \
                 lease_expires_at=null,completed_at=now() where id=$1",
                &[&job.id, &detail],
            )
            .await?;
        if job.kind == "public" {
            transaction
                .execute(
                    "update public.commits set summary_status='failed',summary_error=$2 where id=$1",
                    &[&job.commit_id, &detail],
                )
                .await?;
        }
        if job.attempt_id.is_some() {
            record_attempt(
                &transaction,
                job,
                "failed",
                Some(detail),
                cost.unwrap_or_default(),
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        tracing::error!(error = %error, import_id = %job.import_id, "cannot persist failed job");
    }
}

async fn record_attempt(
    transaction: &tokio_postgres::Transaction<'_>,
    job: &Job,
    status: &str,
    detail: Option<&str>,
    cost: SummarizationCost,
) -> Result<(), tokio_postgres::Error> {
    let attempt_id = job
        .attempt_id
        .expect("upstream work must have a budget reservation");
    transaction
        .execute(
            "update public.summarization_attempts set request_count=$2,unpriced_request_count=$3, \
             cost_usd=$4::double precision::numeric,status=$5,error_detail=$6 where id=$1",
            &[
                &attempt_id,
                &(cost.request_count as i32),
                &(cost.unpriced_request_count as i32),
                &cost.usd,
                &status,
                &detail,
            ],
        )
        .await?;
    Ok(())
}

fn checked_token(
    value: Option<u64>,
) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
    value.map(i64::try_from).transpose().map_err(Into::into)
}

fn summary_origin(origin: aissence::SummaryOrigin) -> &'static str {
    match origin {
        aissence::SummaryOrigin::Model => "model",
        aissence::SummaryOrigin::MetadataFallback => "metadata_fallback",
        aissence::SummaryOrigin::ParseFallback => "parse_fallback",
        aissence::SummaryOrigin::PrivacyFallback => "privacy_fallback",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_reservation_includes_core_prompts_fixed_overhead_and_retries() {
        let (requests, chars) = hosted_usage_ceiling(1, 100, true);
        assert_eq!(requests, 4);
        assert_eq!(chars, ((100 + 8_192) * 2 + 2 * 4_096) * 2);
    }

    #[test]
    fn public_usage_reservation_covers_the_full_prompt_limit() {
        let (requests, chars) = hosted_usage_ceiling(1, HOSTED_PUBLIC_PROMPT_CHARS, true);
        assert_eq!(requests, 4);
        assert_eq!(chars, ((32_000 + 8_192) * 2 + 2 * 4_096) * 2);
    }

    #[test]
    fn usage_reservation_does_not_charge_for_an_unconfigured_fallback() {
        let (requests, chars) = hosted_usage_ceiling(1, 100, false);
        assert_eq!(requests, 2);
        assert_eq!(chars, (100 + 8_192) * 2 + 2 * 4_096);
    }

    #[test]
    fn public_planning_includes_batches_individual_recovery_and_synthesis() {
        let messages = (0..9)
            .map(|_| aissence::MessageSummary {
                kind: aissence::MessageKind::UserWrote,
                subscriber_summary: None,
                content: Some("A short public prompt requiring a safe paraphrase.".into()),
                changes: Vec::new(),
                timestamp: None,
                tokens: None,
            })
            .collect();
        let commit = AissenceCommit {
            commit_hash: "a".repeat(40),
            source_digest: String::new(),
            sanitizer_version: aissence::SANITIZER_VERSION,
            title: "Test".into(),
            body: None,
            date: chrono::DateTime::UNIX_EPOCH,
            stats: aissence::CommitStats::default(),
            files: Vec::new(),
            source: aissence::CommitSource {
                messages,
                agents: Vec::new(),
                tokens: None,
            },
            summary: None,
        };

        // Two eight-entry-limited batches, nine possible individual recovery
        // calls, and one final public-story synthesis.
        assert_eq!(planned_public_requests(&commit), 12);
    }
}
