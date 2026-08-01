use std::time::Duration;

use aissence::{AissenceCommit, CommitSummary, SummarizationCost};
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

pub const HOSTED_REQUEST_RETRIES: usize = 1;
const HOSTED_CORE_INPUT_CHARS: usize = 8_192;
const HOSTED_FIXED_INPUT_CHARS_PER_REQUEST: usize = 4_096;

struct Job {
    import_id: Uuid,
    commit_id: Uuid,
    uploader_id: Uuid,
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

    // An expired processing lease is deliberately failed rather than blindly
    // replayed: the previous worker may already have spent the upstream budget.
    let expired_imports = transaction
        .query(
            "update public.import_commits set status='failed', \
             error_detail='worker lease expired; retry requires a new import', completed_at=now() \
             where status='processing' and lease_expires_at < now() returning import_id,commit_id",
            &[],
        )
        .await?;
    for row in expired_imports {
        transaction
            .execute(
                "update public.commits set summary_status='failed', \
                 summary_error='worker lease expired; retry requires a new import' where id=$1",
                &[&row.get::<_, Uuid>(1)],
            )
            .await?;
        refresh_import(&transaction, row.get(0)).await?;
    }

    let row = transaction
        .query_opt(
            "with candidate as ( \
                select ic.import_id,ic.commit_id from public.import_commits ic \
                where ic.status='queued' order by ic.created_at \
                for update skip locked limit 1 \
             ) \
             update public.import_commits ic set status='processing', attempts=attempts+1, \
                 started_at=now(), lease_expires_at=now()+interval '10 minutes' \
             from candidate c, public.imports i \
             where ic.import_id=c.import_id and ic.commit_id=c.commit_id and i.id=ic.import_id \
             returning ic.import_id,ic.commit_id,i.uploader_id,ic.payload",
            &[],
        )
        .await?;
    if let Some(row) = &row {
        transaction
            .execute(
                "update public.commits set summary_status='processing' where id=$1",
                &[&row.get::<_, Uuid>(1)],
            )
            .await?;
    }
    transaction.commit().await?;
    let Some(row) = row else { return Ok(None) };
    let payload: Value = row.get(3);
    Ok(Some(Job {
        import_id: row.get(0),
        commit_id: row.get(1),
        uploader_id: row.get(2),
        commit: serde_json::from_value(payload)?,
        attempt_id: None,
    }))
}

async fn process(state: &AppState, mut job: Job) {
    let user_message_count = job
        .commit
        .source
        .messages
        .iter()
        .filter(|message| message.kind == aissence::MessageKind::UserWrote)
        .count();
    let source_input_chars = job
        .commit
        .source
        .messages
        .iter()
        .filter(|message| message.kind == aissence::MessageKind::UserWrote)
        .filter_map(|message| message.content.as_deref())
        .map(str::chars)
        .map(Iterator::count)
        .fold(0usize, usize::saturating_add);
    let (request_ceiling, input_char_ceiling) =
        hosted_usage_ceiling(user_message_count, source_input_chars);
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

    let (result, cost) = state
        .summarizer
        .summarize_document_commit_with_cost(&job.commit)
        .await;
    match result {
        Ok(summary) => {
            let mut summary = summary;
            aissence::Redactor::global().redact_summary(&mut summary);
            job.commit.summary = Some(summary.clone());
            if let Err(error) = succeed_job(state, &job, &summary, cost).await {
                tracing::error!(error = %error, import_id = %job.import_id, "cannot persist summary");
            }
        }
        Err(error) => fail_job(state, &job, &error.to_string(), Some(cost)).await,
    }
}

fn hosted_usage_ceiling(user_message_count: usize, source_input_chars: usize) -> (usize, usize) {
    let request_ceiling = user_message_count
        .saturating_add(1)
        .saturating_mul(HOSTED_REQUEST_RETRIES + 1);
    let input_char_ceiling = source_input_chars
        .saturating_add(HOSTED_CORE_INPUT_CHARS)
        .saturating_mul(HOSTED_REQUEST_RETRIES + 1)
        .saturating_add(request_ceiling.saturating_mul(HOSTED_FIXED_INPUT_CHARS_PER_REQUEST));
    (request_ceiling, input_char_ceiling)
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
             (import_id,commit_id,uploader_id,request_count,reserved_request_count,reserved_input_chars,status) \
             values ($1,$2,$3,0,$4,$5,'reserved') returning id",
            &[
                &job.import_id,
                &job.commit_id,
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

async fn succeed_job(
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
            "update public.commits set user_requested=$2,ai_answered=$3,ai_implemented=$4,meta=$5, \
             token_input=$6,token_output=$7,token_cache_read=$8,token_cache_create=$9, \
             summarizer_version=1,summary_status='succeeded',summarized_at=now(),summary_error=null \
             where id=$1",
            &[
                &job.commit_id,
                &summary.user_requested,
                &summary.ai_answered,
                &summary.ai_implemented,
                &summary.meta,
                &token_input,
                &token_output,
                &token_cache_read,
                &token_cache_create,
            ],
        )
        .await?;
    for (sort_order, message) in summary.messages.iter().enumerate() {
        transaction
            .execute(
                "update public.messages set summary=$3 where commit_id=$1 and sort_order=$2",
                &[&job.commit_id, &(sort_order as i32), &message.summary],
            )
            .await?;
    }
    let payload = serde_json::to_value(&job.commit)?;
    transaction
        .execute(
            "update public.import_commits set status='succeeded',payload=$3,error_detail=null, \
             lease_expires_at=null,completed_at=now() where import_id=$1 and commit_id=$2",
            &[&job.import_id, &job.commit_id, &payload],
        )
        .await?;
    record_attempt(&transaction, job, "succeeded", None, cost).await?;
    refresh_import(&transaction, job.import_id).await?;
    transaction.commit().await?;
    Ok(())
}

async fn fail_job(state: &AppState, job: &Job, detail: &str, cost: Option<SummarizationCost>) {
    let outcome: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
        let mut client = state.pool.get().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                "update public.import_commits set status='failed',error_detail=$3, \
                 lease_expires_at=null,completed_at=now() where import_id=$1 and commit_id=$2",
                &[&job.import_id, &job.commit_id, &detail],
            )
            .await?;
        transaction
            .execute(
                "update public.commits set summary_status='failed',summary_error=$2 where id=$1",
                &[&job.commit_id, &detail],
            )
            .await?;
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
        refresh_import(&transaction, job.import_id).await?;
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

async fn refresh_import(
    transaction: &tokio_postgres::Transaction<'_>,
    import_id: Uuid,
) -> Result<(), tokio_postgres::Error> {
    transaction
        .execute(
            "with counts as (select \
                count(*) filter (where status in ('queued','processing')) as pending, \
                count(*) filter (where status='succeeded') as succeeded, \
                count(*) filter (where status='skipped') as skipped, \
                count(*) filter (where status='failed') as failed \
             from public.import_commits where import_id=$1) \
             update public.imports i set \
                summarized_commit_count=counts.succeeded,skipped_commit_count=counts.skipped, \
                status=(case when counts.pending>0 then 'processing' \
                             when counts.failed>0 then 'failed' else 'succeeded' end)::public.import_status, \
                error_detail=case when counts.failed>0 then 'one or more commits failed' else null end, \
                completed_at=case when counts.pending=0 then now() else null end \
             from counts where i.id=$1",
            &[&import_id],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_reservation_includes_core_prompts_fixed_overhead_and_retries() {
        let (requests, chars) = hosted_usage_ceiling(1, 100);
        assert_eq!(requests, 4);
        assert_eq!(chars, (100 + 8_192) * 2 + 4 * 4_096);
    }
}
