use std::collections::{BTreeSet, HashMap, HashSet};

use aissence::document::{AissenceCommit, AissenceDocument, DOCUMENT_VERSION, SANITIZER_VERSION};
use aissence::model::{CommitSummary, MessageKind};
use aissence::remote::{
    AccessTokenResponse, DeviceLoginStart, DeviceTokenRequest, DeviceTokenResponse, ImportAccepted,
    ImportStatus, LogoutRequest, ManifestCommit, ProjectState, RefreshTokenRequest,
    RequestedCommit, ResolveProjectRequest, ServiceConfiguration, SyncPlanRequest,
    SyncPlanResponse, UploadReason, WhoAmI,
};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::{Json, Router, routing};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use tokio_postgres::Transaction;
use uuid::Uuid;

use crate::auth::{AuthUser, SupabaseUser, issue_access_token, random_token, token_hash};
use crate::error::ApiError;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let max_upload_bytes = state.config.max_upload_bytes;
    Router::new()
        .route("/health", routing::get(health))
        .route("/v1/config", routing::get(service_configuration))
        .route("/v1/auth/device", routing::post(start_device_login))
        .route(
            "/v1/auth/device/{request_id}/approve",
            routing::post(approve_device_login),
        )
        .route("/v1/auth/device/token", routing::post(poll_device_login))
        .route("/v1/auth/refresh", routing::post(refresh_access_token))
        .route("/v1/auth/logout", routing::post(logout))
        .route("/v1/whoami", routing::get(whoami))
        .route(
            "/v1/projects/by-source/{source_id}",
            routing::get(project_by_source),
        )
        .route("/v1/projects/resolve", routing::put(resolve_project))
        .route(
            "/v1/projects/{project_id}/sync-plan",
            routing::post(sync_plan),
        )
        .route(
            "/v1/projects/{project_id}/imports",
            routing::post(create_import).layer(upload_body_limit(max_upload_bytes)),
        )
        .route(
            "/v1/projects/{project_id}/browser-imports",
            routing::post(create_browser_import).layer(upload_body_limit(max_upload_bytes)),
        )
        .route("/v1/imports/{import_id}", routing::get(import_status))
        .route(
            "/v1/imports/{import_id}/result",
            routing::get(import_result),
        )
        .layer(axum::middleware::from_fn(no_store_auth_responses))
        .with_state(state)
}

fn upload_body_limit(max_upload_bytes: usize) -> DefaultBodyLimit {
    DefaultBodyLimit::max(max_upload_bytes)
}

async fn no_store_auth_responses(request: axum::extract::Request, next: Next) -> Response {
    let sensitive = request.uri().path().starts_with("/v1/auth/");
    let mut response = next.run(request).await;
    if sensitive {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }
    response
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn service_configuration(State(state): State<AppState>) -> Json<ServiceConfiguration> {
    Json(ServiceConfiguration {
        login_start_endpoint: format!("{}/v1/auth/device", state.config.public_url),
    })
}

async fn start_device_login(
    State(state): State<AppState>,
) -> Result<Json<DeviceLoginStart>, ApiError> {
    let device_code = random_token();
    let browser_code = random_token();
    let client = state.pool.get().await?;
    client
        .execute(
            "update public.cli_login_requests set status='expired' \
             where status in ('pending','approved') and expires_at < now()",
            &[],
        )
        .await?;
    client
        .batch_execute(
            "delete from public.cli_login_requests where expires_at < now()-interval '1 day'; \
             delete from public.cli_refresh_tokens where \
             expires_at < now()-interval '1 day' or revoked_at < now()-interval '1 day'",
        )
        .await?;
    let pending: i64 = client
        .query_one(
            "select count(*) from public.cli_login_requests where status in ('pending','approved')",
            &[],
        )
        .await?
        .get(0);
    if pending >= 10_000 {
        return Err(ApiError::Limit(
            "too many browser login requests are pending".into(),
        ));
    }
    let request_id: Uuid = client
        .query_one(
            "insert into public.cli_login_requests \
             (device_code_hash,browser_code_hash,expires_at) \
             values ($1,$2,now()+interval '10 minutes') returning id",
            &[&token_hash(&device_code), &token_hash(&browser_code)],
        )
        .await?
        .get(0);
    let mut verification = UrlBuilder::new(&format!("{}/cli/authorize", state.config.site_url));
    verification.pair("request_id", &request_id.to_string());
    verification.pair("browser_code", &browser_code);
    Ok(Json(DeviceLoginStart {
        request_id,
        device_code,
        verification_uri_complete: verification.finish(),
        expires_in: 600,
        interval: 2,
    }))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ApproveDeviceRequest {
    browser_code: String,
}

async fn approve_device_login(
    State(state): State<AppState>,
    user: SupabaseUser,
    Path(request_id): Path<Uuid>,
    Json(request): Json<ApproveDeviceRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = state.pool.get().await?;
    let updated = client
        .execute(
            "update public.cli_login_requests set status='approved',user_id=$3,approved_at=now() \
             where id=$1 and browser_code_hash=$2 and status='pending' and expires_at>now()",
            &[&request_id, &token_hash(&request.browser_code), &user.id],
        )
        .await?;
    if updated == 1 {
        Ok(Json(serde_json::json!({"approved": true})))
    } else {
        Err(ApiError::Unauthorized)
    }
}

async fn poll_device_login(
    State(state): State<AppState>,
    Json(request): Json<DeviceTokenRequest>,
) -> Result<Json<DeviceTokenResponse>, ApiError> {
    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;
    let row = transaction
        .query_opt(
            "select status,user_id,expires_at from public.cli_login_requests \
             where id=$1 and device_code_hash=$2 for update",
            &[&request.request_id, &token_hash(&request.device_code)],
        )
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let status: String = row.get(0);
    let expires_at: DateTime<Utc> = row.get(2);
    if expires_at < Utc::now() {
        transaction
            .execute(
                "update public.cli_login_requests set status='expired' where id=$1",
                &[&request.request_id],
            )
            .await?;
        transaction.commit().await?;
        return Ok(Json(DeviceTokenResponse {
            status: "expired".into(),
            access_token: None,
            refresh_token: None,
            expires_in: None,
        }));
    }
    if status == "pending" {
        transaction.commit().await?;
        return Ok(Json(DeviceTokenResponse {
            status,
            access_token: None,
            refresh_token: None,
            expires_in: None,
        }));
    }
    if status != "approved" {
        return Err(ApiError::Unauthorized);
    }
    let user_id: Uuid = row
        .get::<_, Option<Uuid>>(1)
        .ok_or(ApiError::Unauthorized)?;
    let refresh_token = random_token();
    transaction
        .execute(
            "insert into public.cli_refresh_tokens (user_id,token_hash,expires_at) \
             values ($1,$2,now()+make_interval(days=>$3))",
            &[
                &user_id,
                &token_hash(&refresh_token),
                &(state.config.refresh_token_days as i32),
            ],
        )
        .await?;
    transaction
        .execute(
            "update public.cli_login_requests set status='consumed',consumed_at=now() where id=$1",
            &[&request.request_id],
        )
        .await?;
    transaction.commit().await?;
    Ok(Json(DeviceTokenResponse {
        status: "authorized".into(),
        access_token: Some(issue_access_token(&state, user_id)?),
        refresh_token: Some(refresh_token),
        expires_in: Some(state.config.access_token_seconds),
    }))
}

async fn refresh_access_token(
    State(state): State<AppState>,
    Json(request): Json<RefreshTokenRequest>,
) -> Result<Json<AccessTokenResponse>, ApiError> {
    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;
    let Some(row) = transaction
        .query_opt(
            "select t.id,t.user_id,t.revoked_at,t.expires_at from public.cli_refresh_tokens t \
             join auth.users u on u.id=t.user_id where t.token_hash=$1 \
             and u.deleted_at is null and (u.banned_until is null or u.banned_until<now()) \
             and not u.is_anonymous for update of t",
            &[&token_hash(&request.refresh_token)],
        )
        .await?
    else {
        return Err(ApiError::Unauthorized);
    };
    let old_id: Uuid = row.get(0);
    let user_id: Uuid = row.get(1);
    let revoked_at: Option<DateTime<Utc>> = row.get(2);
    let expires_at: DateTime<Utc> = row.get(3);
    if revoked_at.is_some() || expires_at <= Utc::now() {
        // Reuse of a rotated token invalidates every descendant in that token
        // family. This prevents an attacker who races the legitimate CLI from
        // retaining the replacement token.
        transaction
            .execute(
                "with recursive family(id) as (select $1::uuid union all \
                 select t.replaced_by from public.cli_refresh_tokens t join family f on t.id=f.id \
                 where t.replaced_by is not null) \
                 update public.cli_refresh_tokens set revoked_at=coalesce(revoked_at,now()) \
                 where id in (select id from family)",
                &[&old_id],
            )
            .await?;
        transaction.commit().await?;
        return Err(ApiError::Unauthorized);
    }
    let replacement = random_token();
    let replacement_id: Uuid = transaction
        .query_one(
            "insert into public.cli_refresh_tokens (user_id,token_hash,expires_at) \
             values ($1,$2,now()+make_interval(days=>$3)) returning id",
            &[
                &user_id,
                &token_hash(&replacement),
                &(state.config.refresh_token_days as i32),
            ],
        )
        .await?
        .get(0);
    transaction
        .execute(
            "update public.cli_refresh_tokens set revoked_at=now(),last_used_at=now(),replaced_by=$2 where id=$1",
            &[&old_id, &replacement_id],
        )
        .await?;
    transaction.commit().await?;
    Ok(Json(AccessTokenResponse {
        access_token: issue_access_token(&state, user_id)?,
        refresh_token: replacement,
        expires_in: state.config.access_token_seconds,
    }))
}

async fn logout(
    State(state): State<AppState>,
    Json(request): Json<LogoutRequest>,
) -> Result<StatusCode, ApiError> {
    let client = state.pool.get().await?;
    client
        .execute(
            "with recursive family(id) as ( \
             select id from public.cli_refresh_tokens where token_hash=$1 union all \
             select t.replaced_by from public.cli_refresh_tokens t join family f on t.id=f.id \
             where t.replaced_by is not null) \
             update public.cli_refresh_tokens set revoked_at=coalesce(revoked_at,now()) \
             where id in (select id from family)",
            &[&token_hash(&request.refresh_token)],
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

struct UrlBuilder {
    base: String,
    query: Vec<(String, String)>,
}

impl UrlBuilder {
    fn new(base: &str) -> Self {
        Self {
            base: base.into(),
            query: Vec::new(),
        }
    }
    fn pair(&mut self, name: &str, value: &str) {
        self.query.push((name.into(), value.into()));
    }
    fn finish(self) -> String {
        let query = serde_urlencoded::to_string(self.query).expect("URL query serialization");
        format!("{}?{query}", self.base)
    }
}

async fn whoami(user: AuthUser) -> Json<WhoAmI> {
    Json(WhoAmI { user_id: user.id })
}

async fn project_by_source(
    State(state): State<AppState>,
    user: AuthUser,
    Path(source_id): Path<Uuid>,
) -> Result<Json<ProjectState>, ApiError> {
    let client = state.pool.get().await?;
    let row = client
        .query_opt(
            "select p.id, p.sync_revision, p.slug::text, profile.handle::text \
             from public.projects p join public.profiles profile on profile.id=p.owner_id \
             where p.owner_id=$1 and p.source_id=$2",
            &[&user.id, &source_id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(project_state(&state, &row, false)))
}

async fn resolve_project(
    State(state): State<AppState>,
    user: AuthUser,
    Json(request): Json<ResolveProjectRequest>,
) -> Result<Json<ProjectState>, ApiError> {
    if request.source_id.is_nil() {
        return Err(ApiError::Invalid("source_id cannot be nil".into()));
    }
    validate_optional_text(request.name.as_deref(), 120)?;
    validate_optional_text(request.description.as_deref(), 3_000)?;
    validate_optional_text(request.repository.as_deref(), 160)?;
    for value in [
        request.name.as_deref(),
        request.description.as_deref(),
        request.repository.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if aissence::Redactor::global().redact_text(value) != value {
            return Err(ApiError::Invalid(
                "project metadata contains PII or secrets".into(),
            ));
        }
    }
    let mut client = state.pool.get().await?;
    let profile_handle: String = client
        .query_opt(
            "select handle::text from public.profiles where id=$1",
            &[&user.id],
        )
        .await?
        .map(|row| row.get(0))
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "create your creator profile first at {}/dashboard",
                state.config.site_url
            ))
        })?;
    if let Some(row) = client
        .query_opt(
            "select id, sync_revision, slug::text from public.projects \
             where owner_id = $1 and source_id = $2",
            &[&user.id, &request.source_id],
        )
        .await?
    {
        return Ok(Json(project_state_with_handle(
            &state,
            &row,
            &profile_handle,
            false,
        )));
    }

    // Attach an old project with the same normalized repository before making
    // a new draft. This keeps the migration safe for the existing website DB.
    if let Some(repository) = request.repository.as_deref()
        && let Some(row) = client
            .query_opt(
                "update public.projects set source_id = $3, updated_at = now() \
                 where id = (select id from public.projects where owner_id = $1 \
                 and source_id is null and repository_label = $2 limit 1) \
                 returning id, sync_revision, slug::text",
                &[&user.id, &repository, &request.source_id],
            )
            .await?
    {
        return Ok(Json(project_state_with_handle(
            &state,
            &row,
            &profile_handle,
            false,
        )));
    }

    let name = normalized_name(request.name.as_deref());
    let description = request.description.as_deref().unwrap_or("").trim();
    let slug = unique_slug(&mut client, user.id, &name, request.source_id).await?;
    let repository_url = request
        .repository
        .as_deref()
        .map(|repository| format!("https://{repository}"));
    let row = client
        .query_one(
            "insert into public.projects \
             (owner_id, source_id, slug, name, tagline, description, repository_label, \
              repository_url, visibility, source_version) \
             values ($1, $2, $3::public.citext, $4, $5, $6, $7, $8, 'draft', $9) \
             returning id, sync_revision, slug::text",
            &[
                &user.id,
                &request.source_id,
                &slug,
                &name,
                &"",
                &description,
                &request.repository,
                &repository_url,
                &(DOCUMENT_VERSION as i32),
            ],
        )
        .await?;
    Ok(Json(project_state_with_handle(
        &state,
        &row,
        &profile_handle,
        true,
    )))
}

fn project_state(state: &AppState, row: &tokio_postgres::Row, created: bool) -> ProjectState {
    let handle: String = row.get(3);
    project_state_with_handle(state, row, &handle, created)
}

fn project_state_with_handle(
    state: &AppState,
    row: &tokio_postgres::Row,
    handle: &str,
    created: bool,
) -> ProjectState {
    let slug: String = row.get(2);
    ProjectState {
        id: row.get(0),
        sync_revision: row.get(1),
        preview_url: format!("{}/creators/{handle}/{slug}", state.config.site_url),
        created,
    }
}

async fn unique_slug(
    client: &mut deadpool_postgres::Client,
    owner_id: Uuid,
    name: &str,
    source_id: Uuid,
) -> Result<String, ApiError> {
    let mut base = name
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    base = base.trim_matches('-').chars().take(64).collect();
    if base.len() < 2 {
        base = "project".into();
    }
    let exists = client
        .query_opt(
            "select 1 from public.projects where owner_id = $1 and slug = $2::public.citext",
            &[&owner_id, &base],
        )
        .await?
        .is_some();
    if exists {
        Ok(format!("{base}-{}", &source_id.simple().to_string()[..8]))
    } else {
        Ok(base)
    }
}

fn normalized_name(name: Option<&str>) -> String {
    let value = name.unwrap_or("Aissence project").trim();
    if value.is_empty() {
        "Aissence project".into()
    } else {
        value.chars().take(120).collect()
    }
}

async fn sync_plan(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project_id): Path<Uuid>,
    Json(request): Json<SyncPlanRequest>,
) -> Result<Json<SyncPlanResponse>, ApiError> {
    if request.file_version != DOCUMENT_VERSION {
        return Err(ApiError::Invalid(format!(
            "unsupported file version {}; expected {DOCUMENT_VERSION}",
            request.file_version
        )));
    }
    if request.commits.len() > state.config.max_commits_per_upload * 20 {
        return Err(ApiError::Limit("manifest contains too many commits".into()));
    }
    let client = state.pool.get().await?;
    let revision = project_revision(&client, user.id, project_id).await?;
    let rows = client
        .query(
            "select commit_hash, source_digest, summary_status::text, public_prompt_summaries, \
                    coalesce(summarizer_version,0) \
             from public.commits where project_id = $1",
            &[&project_id],
        )
        .await?;
    type RemoteCommitState = (Option<String>, Option<String>, bool, i32);
    let remote: HashMap<String, RemoteCommitState> = rows
        .into_iter()
        .map(|row| (row.get(0), (row.get(1), row.get(2), row.get(3), row.get(4))))
        .collect();
    let mut seen = HashSet::new();
    let mut upload = Vec::new();
    let mut unchanged_count = 0;
    for commit in &request.commits {
        validate_manifest_commit(commit)?;
        if !seen.insert(commit.commit_hash.clone()) {
            return Err(ApiError::Invalid(format!(
                "duplicate commit {} in manifest",
                commit.commit_hash
            )));
        }
        match remote.get(&commit.commit_hash) {
            None => upload.push(RequestedCommit {
                commit_hash: commit.commit_hash.clone(),
                reason: UploadReason::Missing,
            }),
            Some((digest, _status, public_prompts, summary_version))
                if stored_commit_satisfies_upload(
                    digest.as_deref() == Some(commit.source_digest.as_str()),
                    commit.all_prompts,
                    *public_prompts,
                    commit.summarizer_version,
                    *summary_version,
                ) =>
            {
                unchanged_count += 1;
            }
            Some(_) => upload.push(RequestedCommit {
                commit_hash: commit.commit_hash.clone(),
                reason: UploadReason::Changed,
            }),
        }
    }
    let remote_only = remote
        .keys()
        .filter(|hash| !seen.contains(*hash))
        .cloned()
        .collect();
    Ok(Json(SyncPlanResponse {
        upload,
        unchanged_count,
        remote_only,
        project_revision: revision,
    }))
}

fn validate_manifest_commit(commit: &ManifestCommit) -> Result<(), ApiError> {
    if !valid_commit_hash(&commit.commit_hash) {
        return Err(ApiError::Invalid(format!(
            "invalid commit hash {}",
            commit.commit_hash
        )));
    }
    if !valid_digest(&commit.source_digest) {
        return Err(ApiError::Invalid(format!(
            "invalid source digest for {}",
            commit.commit_hash
        )));
    }
    Ok(())
}

/// Whether the server already has everything represented by an incoming
/// manifest entry. A source-only manifest deliberately has summary version 0:
/// its source can be unchanged even while a draft has not generated summaries.
fn stored_commit_satisfies_upload(
    digest_matches: bool,
    incoming_all_prompts: bool,
    stored_public_prompts: bool,
    incoming_summary_version: u32,
    stored_summary_version: i32,
) -> bool {
    digest_matches
        && (!incoming_all_prompts || stored_public_prompts)
        && (incoming_summary_version == 0
            || i64::from(stored_summary_version) >= i64::from(incoming_summary_version))
}

async fn create_import(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    create_import_for_user(state, user.id, project_id, headers, body).await
}

async fn create_browser_import(
    State(state): State<AppState>,
    user: SupabaseUser,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    create_import_for_user(state, user.id, project_id, headers, body).await
}

async fn create_import_for_user(
    state: AppState,
    user_id: Uuid,
    project_id: Uuid,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type.starts_with("application/json")
        && !content_type.starts_with("application/vnd.aissence+json")
    {
        return Err(ApiError::UnsupportedMediaType);
    }
    if headers.get(header::CONTENT_ENCODING).is_some() {
        return Err(ApiError::UnsupportedMediaType);
    }
    if body.len() > state.config.max_upload_bytes {
        return Err(ApiError::PayloadTooLarge);
    }
    let mut document = parse_upload(&state.config, &body)?;
    let checksum = format!("sha256:{}", hex::encode(Sha256::digest(&body)));
    let accepted = ingest_document(&state, user_id, project_id, checksum, &mut document).await?;
    Ok((StatusCode::ACCEPTED, Json(accepted)))
}

fn parse_upload(config: &crate::config::Config, body: &[u8]) -> Result<AissenceDocument, ApiError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| ApiError::Malformed(error.to_string()))?;
    let mut document: AissenceDocument =
        serde_json::from_value(value).map_err(|error| ApiError::Invalid(error.to_string()))?;
    normalize_legacy_empty_prompt_summaries(&mut document);
    validate_document(config, &document)?;
    // Client-provided digests are verified before any database write.
    for commit in &document.commits {
        if !commit.digest_is_valid() {
            return Err(ApiError::Invalid(format!(
                "source digest mismatch for {}",
                commit.commit_hash
            )));
        }
    }
    if !aissence::Redactor::global().document_is_clean(&document) {
        return Err(ApiError::Invalid(
            "document contains PII or secrets; sanitize it with the current Aissence client".into(),
        ));
    }

    Ok(document)
}

/// Older `--all-prompts` builds could copy a whitespace-only retained prompt
/// into an empty summary. Repair only that exact shape; an empty summary for a
/// non-empty prompt remains invalid.
fn normalize_legacy_empty_prompt_summaries(document: &mut AissenceDocument) {
    for commit in &mut document.commits {
        let Some(summary) = &mut commit.summary else {
            continue;
        };
        for (source, generated) in commit.source.messages.iter().zip(&mut summary.messages) {
            if source.kind == MessageKind::UserWrote
                && source
                    .content
                    .as_deref()
                    .is_none_or(|content| content.trim().is_empty())
                && generated.kind == MessageKind::UserWrote
                && generated
                    .subscriber_summary
                    .as_deref()
                    .is_some_and(|text| text.trim().is_empty())
            {
                generated.subscriber_summary =
                    Some(aissence::summarize::EMPTY_USER_SUMMARY.to_string());
            }
        }
    }
}

fn validate_document(
    config: &crate::config::Config,
    document: &AissenceDocument,
) -> Result<(), ApiError> {
    document.validate_version().map_err(ApiError::Invalid)?;
    if document.project.source_id.is_nil() {
        return Err(ApiError::Invalid("project source_id cannot be nil".into()));
    }
    validate_optional_text(document.project.name.as_deref(), config.max_entry_chars)?;
    validate_optional_text(
        document.project.repository.as_deref(),
        config.max_entry_chars,
    )?;
    if document.commits.is_empty() {
        return Err(ApiError::Invalid("document contains no commits".into()));
    }
    if document.commits.len() > config.max_commits_per_upload {
        return Err(ApiError::Limit("upload contains too many commits".into()));
    }
    let mut hashes = HashSet::new();
    for commit in &document.commits {
        validate_text(&commit.title, config.max_entry_chars)?;
        validate_optional_text(commit.body.as_deref(), config.max_entry_chars)?;
        validate_token_count(commit.source.tokens)?;
        if !valid_commit_hash(&commit.commit_hash) {
            return Err(ApiError::Invalid(format!(
                "invalid commit hash {}",
                commit.commit_hash
            )));
        }
        if !hashes.insert(commit.commit_hash.clone()) {
            return Err(ApiError::Invalid(format!(
                "duplicate commit {}",
                commit.commit_hash
            )));
        }
        if commit.sanitizer_version != SANITIZER_VERSION {
            return Err(ApiError::Invalid(format!(
                "unsupported sanitizer version {}",
                commit.sanitizer_version
            )));
        }
        if commit.source.messages.len() > config.max_messages_per_commit {
            return Err(ApiError::Limit(format!(
                "commit {} contains too many messages",
                commit.commit_hash
            )));
        }
        if commit.files.len() > config.max_messages_per_commit
            || commit.source.agents.len() > config.max_messages_per_commit
        {
            return Err(ApiError::Limit(format!(
                "commit {} contains too many file or agent entries",
                commit.commit_hash
            )));
        }
        for file in &commit.files {
            validate_text(&file.path, config.max_entry_chars)?;
            validate_optional_text(file.previous_path.as_deref(), config.max_entry_chars)?;
        }
        for agent in &commit.source.agents {
            validate_text(&agent.provider, config.max_entry_chars)?;
            validate_optional_text(agent.model.as_deref(), config.max_entry_chars)?;
            validate_token_count(agent.tokens)?;
        }
        for message in &commit.source.messages {
            validate_token_count(message.tokens)?;
            if message.subscriber_summary.is_some() {
                return Err(ApiError::Invalid(format!(
                    "commit {} includes subscriber text in source.messages",
                    commit.commit_hash
                )));
            }
            let correct_shape = match message.kind {
                MessageKind::UserWrote | MessageKind::AiWrote => {
                    message.content.is_some() && message.changes.is_empty()
                }
                MessageKind::AiChanged => message.content.is_none() && !message.changes.is_empty(),
            };
            if !correct_shape {
                return Err(ApiError::Invalid(format!(
                    "commit {} contains an invalid source message shape",
                    commit.commit_hash
                )));
            }
            if message
                .content
                .as_deref()
                .is_some_and(|content| content.chars().count() > config.max_entry_chars)
            {
                return Err(ApiError::Limit(format!(
                    "commit {} contains an oversized message",
                    commit.commit_hash
                )));
            }
            for change in &message.changes {
                validate_text(&change.tool, config.max_entry_chars)?;
                validate_text(&change.file, config.max_entry_chars)?;
                if change.diff.chars().count() > config.max_entry_chars {
                    return Err(ApiError::Limit(format!(
                        "commit {} contains an oversized change",
                        commit.commit_hash
                    )));
                }
            }
        }
        validate_summary(commit, config.max_entry_chars)?;
    }
    Ok(())
}

fn validate_summary(commit: &AissenceCommit, max_entry_chars: usize) -> Result<(), ApiError> {
    let Some(summary) = &commit.summary else {
        return Ok(());
    };
    let total_chars = summary.public.summary.chars().count()
        + summary
            .public
            .beats
            .iter()
            .map(|beat| beat.label.chars().count() + beat.text.chars().count())
            .sum::<usize>();
    if summary.summarizer_version != aissence::SUMMARIZER_VERSION
        || total_chars > 32_000
        || summary.public.subline.chars().count() > 1_000
        || summary.messages.len() != commit.source.messages.len()
    {
        return Err(ApiError::Invalid(format!(
            "invalid local summary for {}",
            commit.commit_hash
        )));
    }
    validate_token_count(summary.tokens)?;
    for message in &summary.messages {
        validate_optional_text(message.subscriber_summary.as_deref(), max_entry_chars)?;
        if message
            .subscriber_summary
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err(ApiError::Invalid(format!(
                "local summary contains an empty prompt summary for {}",
                commit.commit_hash
            )));
        }
        validate_token_count(message.tokens)?;
        let correct_shape = match message.kind {
            MessageKind::UserWrote => true,
            MessageKind::AiWrote | MessageKind::AiChanged => message.subscriber_summary.is_none(),
        };
        if !correct_shape {
            return Err(ApiError::Invalid(format!(
                "local summary message shape differs for {}",
                commit.commit_hash
            )));
        }
    }
    for agent in &summary.agents {
        validate_text(&agent.provider, max_entry_chars)?;
        validate_optional_text(agent.model.as_deref(), max_entry_chars)?;
        validate_token_count(agent.tokens)?;
    }
    let mut failed_prompt_indices = std::collections::HashSet::new();
    for failure in &summary.prompt_failures {
        validate_text(&failure.model, max_entry_chars)?;
        validate_text(&failure.error, max_entry_chars)?;
        if failure.message_index >= commit.source.messages.len()
            || commit.source.messages[failure.message_index].kind != MessageKind::UserWrote
            || summary.messages[failure.message_index]
                .subscriber_summary
                .is_some()
            || !failed_prompt_indices.insert(failure.message_index)
        {
            return Err(ApiError::Invalid(format!(
                "invalid prompt failure metadata for {}",
                commit.commit_hash
            )));
        }
    }
    if summary
        .messages
        .iter()
        .zip(&commit.source.messages)
        .any(|(summary, source)| summary.kind != source.kind)
    {
        return Err(ApiError::Invalid(format!(
            "local summary message shape differs for {}",
            commit.commit_hash
        )));
    }
    let mut seen_beats = std::collections::HashSet::new();
    if summary.public.summary.trim().is_empty()
        || summary.public.beats.len() > 3
        || aissence::summarize::public_story_has_verbatim_prompt(
            &summary.public,
            &commit.source.messages,
        )
        || summary.public.beats.iter().any(|beat| {
            beat.message_index >= commit.source.messages.len()
                || commit.source.messages[beat.message_index].kind != MessageKind::UserWrote
                || beat.label.trim().is_empty()
                || beat.text.trim().is_empty()
                || !seen_beats.insert(beat.message_index)
        })
    {
        return Err(ApiError::Invalid(format!(
            "invalid public story for {}",
            commit.commit_hash
        )));
    }
    let all_prompt_summaries = summary
        .messages
        .iter()
        .any(|message| message.kind == MessageKind::UserWrote)
        && summary.messages.iter().all(|message| {
            message.kind != MessageKind::UserWrote || message.subscriber_summary.is_some()
        });
    if all_prompt_summaries
        && summary
            .messages
            .iter()
            .zip(&commit.source.messages)
            .any(|(generated, source)| {
                generated.kind == MessageKind::UserWrote
                    && generated.subscriber_summary.as_deref().is_some_and(|text| {
                        let public = aissence::PublicStory {
                            subline: String::new(),
                            summary: text.to_string(),
                            beats: Vec::new(),
                            origin: aissence::SummaryOrigin::Model,
                        };
                        aissence::summarize::public_story_has_verbatim_prompt(
                            &public,
                            std::slice::from_ref(source),
                        )
                    })
            })
    {
        return Err(ApiError::Invalid(format!(
            "public prompt summary is too close to its source for {}",
            commit.commit_hash
        )));
    }
    Ok(())
}

fn summary_origin_name(origin: aissence::SummaryOrigin) -> &'static str {
    match origin {
        aissence::SummaryOrigin::Model => "model",
        aissence::SummaryOrigin::MetadataFallback => "metadata_fallback",
        aissence::SummaryOrigin::ParseFallback => "parse_fallback",
        aissence::SummaryOrigin::PrivacyFallback => "privacy_fallback",
    }
}

fn validate_text(value: &str, max_chars: usize) -> Result<(), ApiError> {
    if value.chars().count() > max_chars {
        Err(ApiError::Limit(
            "document contains an oversized text field".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_text(value: Option<&str>, max_chars: usize) -> Result<(), ApiError> {
    value.map_or(Ok(()), |value| validate_text(value, max_chars))
}

fn validate_token_count(tokens: Option<aissence::TokenCount>) -> Result<(), ApiError> {
    let Some(tokens) = tokens else { return Ok(()) };
    u64_to_i64(tokens.input)?;
    u64_to_i64(tokens.output)?;
    if let Some(value) = tokens.cache_read {
        u64_to_i64(value)?;
    }
    if let Some(value) = tokens.cache_create {
        u64_to_i64(value)?;
    }
    Ok(())
}

fn valid_commit_hash(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

async fn ingest_document(
    state: &AppState,
    user_id: Uuid,
    project_id: Uuid,
    checksum: String,
    document: &mut AissenceDocument,
) -> Result<ImportAccepted, ApiError> {
    let mut client = state.pool.get().await?;
    let transaction = client.transaction().await?;
    let project = transaction
        .query_opt(
            "select source_id, visibility::text from public.projects \
             where id = $1 and owner_id = $2 for update",
            &[&project_id, &user_id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let source_id: Option<Uuid> = project.get(0);
    let is_published = project.get::<_, String>(1) == "published";
    if source_id.is_some_and(|source_id| source_id != document.project.source_id) {
        return Err(ApiError::Conflict(
            "document belongs to a different project".into(),
        ));
    }
    if source_id.is_none() {
        transaction
            .execute(
                "update public.projects set source_id=$2 where id=$1",
                &[&project_id, &document.project.source_id],
            )
            .await?;
    }
    if let Some(row) = transaction
        .query_opt(
            "select id, status::text, submitted_commit_count, summarized_commit_count, \
             skipped_commit_count from public.imports where project_id = $1 and checksum = $2",
            &[&project_id, &checksum],
        )
        .await?
    {
        let existing_import_id: Uuid = row.get(0);
        let queued_commit_count: i64 = transaction
            .query_one(
                "select count(distinct commit_id) from public.summary_jobs \
                 where import_id=$1 and status in ('queued','processing')",
                &[&existing_import_id],
            )
            .await?
            .get(0);
        return Ok(ImportAccepted {
            import_id: existing_import_id,
            status: row.get(1),
            submitted_commit_count: row.get::<_, i32>(2) as usize,
            queued_commit_count: queued_commit_count as usize,
            skipped_commit_count: row.get::<_, i32>(4) as usize,
        });
    }

    let import_id: Uuid = transaction
        .query_one(
            "insert into public.imports \
             (project_id, uploader_id, checksum, source_version, status, commit_count, \
              submitted_commit_count) \
             values ($1, $2, $3, $4, 'processing', $5, $5) returning id",
            &[
                &project_id,
                &user_id,
                &checksum,
                &(DOCUMENT_VERSION as i32),
                &(document.commits.len() as i32),
            ],
        )
        .await?
        .get(0);

    let mut queued = 0usize;
    let mut skipped = 0usize;
    let mut locally_summarized = 0usize;
    let has_subscriber: bool = is_published
        && transaction
            .query_one(
                "select exists(select 1 from public.creator_subscriptions \
                 where creator_id=$1 and status in ('active','trialing') \
                 and (current_period_end is null or current_period_end > now()))",
                &[&user_id],
            )
            .await?
            .get(0);
    for (sort_order, commit) in document.commits.iter_mut().enumerate() {
        let incoming_all_prompts = commit.summary.as_ref().is_some_and(|summary| {
            summary
                .messages
                .iter()
                .any(|message| message.kind == MessageKind::UserWrote)
                && summary.messages.iter().all(|message| {
                    message.kind != MessageKind::UserWrote || message.subscriber_summary.is_some()
                })
        });
        let incoming_summary_version = commit
            .summary
            .as_ref()
            .map(|summary| summary.summarizer_version)
            .unwrap_or(0);
        let existing = transaction
            .query_opt(
                "select c.id, c.source_digest, c.summary_status::text, \
                 exists(select 1 from public.summary_jobs sj where sj.commit_id=c.id \
                        and sj.status in ('queued','processing')), \
                 c.public_prompt_summaries, coalesce(c.summarizer_version,0) \
                 from public.commits c \
                 where project_id = $1 and commit_hash = $2",
                &[&project_id, &commit.commit_hash],
            )
            .await?;
        if existing.as_ref().is_some_and(|row| {
            (row.get::<_, Option<String>>(1).as_deref() != Some(commit.source_digest.as_str())
                || incoming_all_prompts
                || incoming_summary_version > 0)
                && row.get::<_, bool>(3)
        }) {
            return Err(ApiError::Conflict(format!(
                "commit {} is already being summarized; retry after it finishes",
                commit.commit_hash
            )));
        }
        let unchanged = existing.as_ref().is_some_and(|row| {
            stored_commit_satisfies_upload(
                row.get::<_, Option<String>>(1).as_deref() == Some(commit.source_digest.as_str()),
                incoming_all_prompts,
                row.get::<_, bool>(4),
                incoming_summary_version,
                row.get::<_, i32>(5),
            )
        });
        if unchanged {
            let commit_id: Uuid = existing.expect("checked existing").get(0);
            transaction
                .execute(
                    "insert into public.import_commits \
                     (import_id, commit_id, status, payload, completed_at) \
                     values ($1, $2, 'skipped', $3, now())",
                    &[
                        &import_id,
                        &commit_id,
                        &serde_json::to_value(&commit)
                            .map_err(|e| ApiError::Internal(e.to_string()))?,
                    ],
                )
                .await?;
            skipped += 1;
            continue;
        }

        let summary = commit.summary.as_ref();
        let prompt_count = commit
            .source
            .messages
            .iter()
            .filter(|message| {
                message.kind == MessageKind::UserWrote
                    && message
                        .content
                        .as_deref()
                        .is_some_and(|content| !content.trim().is_empty())
            })
            .count();
        let reply_count = commit
            .source
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::AiWrote)
            .count();
        let change_message_count = commit
            .source
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::AiChanged)
            .count();
        let summarized_prompts = summary
            .map(|summary| {
                summary
                    .messages
                    .iter()
                    .filter(|message| {
                        message.kind == MessageKind::UserWrote
                            && message.subscriber_summary.is_some()
                    })
                    .count()
            })
            .unwrap_or(0);
        let public_prompt_summaries = prompt_count > 0 && summarized_prompts == prompt_count;
        let status = if summary.is_some() {
            "succeeded"
        } else if is_published && prompt_count > 0 {
            "queued"
        } else {
            "skipped"
        };
        let commit_id: Uuid = transaction
            .query_one(
                "insert into public.commits \
                 (project_id, commit_hash, title, committed_at, files_changed, lines_added, \
                  lines_deleted, public_story, public_subline, public_beats, summary_origin, token_input, token_output, \
                  token_cache_read, token_cache_create, sort_order, source_digest, \
                  sanitizer_version, summarizer_version, summary_status, summarized_at, summary_error, \
                  prompt_count,reply_count,change_message_count,public_prompt_summaries) \
                 values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::bigint,$13::bigint, \
                         $14::bigint,$15::bigint,$16,$17,$18,$19, \
                         $20::text::public.summary_job_status, \
                         case when $20::text = 'succeeded' then now() end, null, \
                         $21,$22,$23,$24) \
                 on conflict (project_id, commit_hash) do update set \
                  title=excluded.title, committed_at=excluded.committed_at, files_changed=excluded.files_changed, \
                  lines_added=excluded.lines_added, lines_deleted=excluded.lines_deleted, \
                  public_story=excluded.public_story, public_subline=excluded.public_subline, \
                  summary_origin=excluded.summary_origin, \
                  public_beats=excluded.public_beats, token_input=excluded.token_input, \
                  token_output=excluded.token_output, token_cache_read=excluded.token_cache_read, \
                  token_cache_create=excluded.token_cache_create, sort_order=excluded.sort_order, \
                  source_digest=excluded.source_digest, sanitizer_version=excluded.sanitizer_version, \
                  summarizer_version=excluded.summarizer_version, summary_status=excluded.summary_status, \
                  summarized_at=excluded.summarized_at, summary_error=null, \
                  prompt_count=excluded.prompt_count,reply_count=excluded.reply_count, \
                  change_message_count=excluded.change_message_count, \
                  public_prompt_summaries=excluded.public_prompt_summaries \
                 returning id",
                &[
                    &project_id,
                    &commit.commit_hash,
                    &commit.title,
                    &commit.date,
                    &u64_to_i64(commit.stats.files_changed)?,
                    &u64_to_i64(commit.stats.lines_added)?,
                    &u64_to_i64(commit.stats.lines_deleted)?,
                    &summary.map(|s| s.public.summary.as_str()).unwrap_or(""),
                    &"",
                    &summary
                        .map(|s| serde_json::to_value(&s.public.beats))
                        .transpose()
                        .map_err(|error| ApiError::Internal(error.to_string()))?
                        .unwrap_or_else(|| serde_json::json!([])),
                    &summary
                        .map(|summary| summary_origin_name(summary.public.origin))
                        .unwrap_or("pending"),
                    &optional_token(summary.and_then(|s| s.tokens), commit.source.tokens, |t| t.input)?,
                    &optional_token(summary.and_then(|s| s.tokens), commit.source.tokens, |t| t.output)?,
                    &optional_optional_token(summary.and_then(|s| s.tokens), commit.source.tokens, |t| t.cache_read)?,
                    &optional_optional_token(summary.and_then(|s| s.tokens), commit.source.tokens, |t| t.cache_create)?,
                    &(sort_order as i32),
                    &commit.source_digest,
                    &(commit.sanitizer_version as i32),
                    &summary.map(|summary| summary.summarizer_version as i32),
                    &status,
                    &(prompt_count as i32),
                    &(reply_count as i32),
                    &(change_message_count as i32),
                    &public_prompt_summaries,
                ],
            )
            .await?
            .get(0);

        transaction
            .execute(
                "delete from public.commit_agents where commit_id = $1",
                &[&commit_id],
            )
            .await?;
        transaction
            .execute(
                "delete from public.messages where commit_id = $1",
                &[&commit_id],
            )
            .await?;
        insert_source_rows(&transaction, commit_id, commit, summary).await?;
        let payload =
            serde_json::to_value(&commit).map_err(|error| ApiError::Internal(error.to_string()))?;
        transaction
            .execute(
                "insert into public.import_commits \
                 (import_id, commit_id, status, payload, completed_at) \
                 values ($1,$2,'succeeded',$3,now())",
                &[&import_id, &commit_id, &payload],
            )
            .await?;
        if summary.is_some() {
            locally_summarized += 1;
        } else if is_published && prompt_count > 0 {
            transaction
                .execute(
                    "insert into public.summary_jobs (import_id,commit_id,kind,payload) \
                     values ($1,$2,'public',$3) \
                     on conflict (commit_id,kind) do update set import_id=excluded.import_id, \
                     payload=excluded.payload,status='queued',attempts=0,error_detail=null, \
                     lease_expires_at=null,started_at=null,completed_at=null",
                    &[&import_id, &commit_id, &payload],
                )
                .await?;
            queued += 1;
        }
        if has_subscriber && prompt_count > summarized_prompts {
            transaction
                .execute(
                    "insert into public.summary_jobs (import_id,commit_id,kind,payload) \
                     values ($1,$2,'subscriber',$3) \
                     on conflict (commit_id,kind) do update set import_id=excluded.import_id, \
                     payload=excluded.payload,status='queued',attempts=0,error_detail=null, \
                     lease_expires_at=null,started_at=null,completed_at=null",
                    &[&import_id, &commit_id, &payload],
                )
                .await?;
        }
    }

    let import_status = if document.commits.len() > skipped {
        "succeeded"
    } else {
        "noop"
    };
    transaction
        .execute(
            "update public.imports set status=$2::text::public.import_status, skipped_commit_count=$3, \
             summarized_commit_count=$4, completed_at=now() \
             where id=$1",
            &[
                &import_id,
                &import_status,
                &(skipped as i32),
                &(locally_summarized as i32),
            ],
        )
        .await?;
    transaction
        .execute(
            "update public.projects set source_version=$2, import_checksum=$3, \
             sync_revision=sync_revision+1, last_imported_at=now(), updated_at=now() where id=$1",
            &[&project_id, &(DOCUMENT_VERSION as i32), &checksum],
        )
        .await?;
    transaction.commit().await?;
    Ok(ImportAccepted {
        import_id,
        status: import_status.into(),
        submitted_commit_count: document.commits.len(),
        queued_commit_count: queued,
        skipped_commit_count: skipped,
    })
}

async fn insert_source_rows(
    transaction: &Transaction<'_>,
    commit_id: Uuid,
    commit: &AissenceCommit,
    summary: Option<&CommitSummary>,
) -> Result<(), ApiError> {
    for (sort_order, agent) in commit.source.agents.iter().enumerate() {
        transaction
            .execute(
                "insert into public.commit_agents (commit_id,provider,model,sort_order) values ($1,$2,$3,$4)",
                &[&commit_id, &agent.provider, &agent.model, &(sort_order as i32)],
            )
            .await?;
    }
    for (sort_order, message) in commit.source.messages.iter().enumerate() {
        let generated = summary.and_then(|summary| summary.messages.get(sort_order));
        let kind = kind_name(message.kind);
        let tokens = generated
            .and_then(|message| message.tokens)
            .or(message.tokens);
        let token_input = tokens.map(|tokens| u64_to_i64(tokens.input)).transpose()?;
        let token_output = tokens.map(|tokens| u64_to_i64(tokens.output)).transpose()?;
        let token_cache_read = tokens
            .and_then(|tokens| tokens.cache_read)
            .map(u64_to_i64)
            .transpose()?;
        let token_cache_create = tokens
            .and_then(|tokens| tokens.cache_create)
            .map(u64_to_i64)
            .transpose()?;
        let changed_file_count = message
            .changes
            .iter()
            .map(|change| change.file.as_str())
            .collect::<HashSet<_>>()
            .len();
        let message_id: Uuid = transaction
            .query_one(
                "insert into public.messages \
                 (commit_id,kind,occurred_at,token_input,token_output,token_cache_read, \
                  token_cache_create,sort_order,changed_file_count) \
                 values ($1,$2::text::public.message_kind,$3,$4::bigint,$5::bigint, \
                         $6::bigint,$7::bigint,$8,$9) returning id",
                &[
                    &commit_id,
                    &kind,
                    &message.timestamp,
                    &token_input,
                    &token_output,
                    &token_cache_read,
                    &token_cache_create,
                    &(sort_order as i32),
                    &(changed_file_count as i32),
                ],
            )
            .await?
            .get(0);
        if let Some(subscriber_summary) =
            generated.and_then(|message| message.subscriber_summary.as_deref())
        {
            transaction
                .execute(
                    "insert into public.message_summaries \
                     (message_id,summary,summarizer_version) values ($1,$2,$3)",
                    &[
                        &message_id,
                        &subscriber_summary,
                        &(aissence::SUMMARIZER_VERSION as i32),
                    ],
                )
                .await?;
        }
        if let Some(content) = &message.content {
            transaction
                .execute(
                    "insert into public.message_sources (message_id,content) values ($1,$2)",
                    &[&message_id, &content],
                )
                .await?;
        }
        for (change_order, change) in message.changes.iter().enumerate() {
            transaction
                .execute(
                    "insert into public.changes (message_id,tool,file_path,diff,sort_order) values ($1,$2,$3,$4,$5)",
                    &[&message_id, &change.tool, &change.file, &change.diff, &(change_order as i32)],
                )
                .await?;
        }
    }
    Ok(())
}

fn kind_name(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::UserWrote => "UserWrote",
        MessageKind::AiWrote => "AiWrote",
        MessageKind::AiChanged => "AiChanged",
    }
}

fn u64_to_i64(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| ApiError::Invalid("numeric value exceeds Postgres bigint".into()))
}

fn optional_token(
    summary: Option<aissence::TokenCount>,
    source: Option<aissence::TokenCount>,
    field: impl Fn(aissence::TokenCount) -> u64,
) -> Result<Option<i64>, ApiError> {
    summary.or(source).map(field).map(u64_to_i64).transpose()
}

fn optional_optional_token(
    summary: Option<aissence::TokenCount>,
    source: Option<aissence::TokenCount>,
    field: impl Fn(aissence::TokenCount) -> Option<u64>,
) -> Result<Option<i64>, ApiError> {
    summary
        .or(source)
        .and_then(field)
        .map(u64_to_i64)
        .transpose()
}

async fn import_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path(import_id): Path<Uuid>,
) -> Result<Json<ImportStatus>, ApiError> {
    let client = state.pool.get().await?;
    let row = client
        .query_opt(
            "select i.id,i.status::text,i.submitted_commit_count,i.summarized_commit_count, \
             i.skipped_commit_count,i.error_detail, \
             (select count(*) from public.import_commits ic where ic.import_id=i.id and ic.status='failed') \
             from public.imports i join public.projects p on p.id=i.project_id \
             where i.id=$1 and p.owner_id=$2",
            &[&import_id, &user.id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(ImportStatus {
        import_id: row.get(0),
        status: row.get(1),
        submitted_commit_count: row.get(2),
        summarized_commit_count: row.get(3),
        skipped_commit_count: row.get(4),
        error_detail: row.get(5),
        failed_commit_count: row.get(6),
    }))
}

async fn import_result(
    State(state): State<AppState>,
    user: AuthUser,
    Path(import_id): Path<Uuid>,
) -> Result<Json<AissenceDocument>, ApiError> {
    let client = state.pool.get().await?;
    let row = client
        .query_opt(
            "select p.source_id,p.name,p.repository_label,i.status::text \
             from public.imports i join public.projects p on p.id=i.project_id \
             where i.id=$1 and p.owner_id=$2",
            &[&import_id, &user.id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let status: String = row.get(3);
    if status == "processing" {
        return Err(ApiError::Conflict("import is still processing".into()));
    }
    let commits = client
        .query(
            "select payload from public.import_commits where import_id=$1 \
             and status in ('succeeded','skipped') order by created_at,commit_id",
            &[&import_id],
        )
        .await?
        .into_iter()
        .filter_map(|row| row.get::<_, Option<serde_json::Value>>(0))
        .map(|value| {
            serde_json::from_value(value).map_err(|error| ApiError::Internal(error.to_string()))
        })
        .collect::<Result<Vec<AissenceCommit>, ApiError>>()?;
    Ok(Json(AissenceDocument {
        version: DOCUMENT_VERSION,
        project: aissence::AissenceProject {
            source_id: row.get(0),
            name: row.get(1),
            repository: row.get(2),
        },
        commits,
        ignored: BTreeSet::new(),
    }))
}

async fn project_revision(
    client: &deadpool_postgres::Client,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<i64, ApiError> {
    client
        .query_opt(
            "select sync_revision from public.projects where id=$1 and owner_id=$2",
            &[&project_id, &user_id],
        )
        .await?
        .map(|row| row.get(0))
        .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_config() -> crate::config::Config {
        crate::config::Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
            supabase_jwt_secret: "x".repeat(32),
            token_secret: "y".repeat(32),
            token_issuer: "test".into(),
            token_audience: "test".into(),
            public_url: "http://127.0.0.1:8787".into(),
            site_url: "http://127.0.0.1:5173".into(),
            access_token_seconds: 900,
            refresh_token_days: 30,
            max_upload_bytes: 1_000_000,
            max_commits_per_upload: 10,
            max_messages_per_commit: 10,
            max_entry_chars: 10_000,
            daily_user_request_limit: 100,
            daily_global_request_limit: 1_000,
            daily_user_input_chars: 1_000_000,
            daily_global_input_chars: 10_000_000,
        }
    }

    fn sample_document() -> AissenceDocument {
        let mut commit = AissenceCommit {
            commit_hash: "a".repeat(40),
            source_digest: String::new(),
            sanitizer_version: SANITIZER_VERSION,
            title: "Test commit".into(),
            body: None,
            date: DateTime::UNIX_EPOCH,
            stats: aissence::CommitStats::default(),
            files: Vec::new(),
            source: aissence::CommitSource {
                messages: vec![aissence::MessageSummary {
                    kind: MessageKind::UserWrote,
                    subscriber_summary: None,
                    content: Some("Please update the documentation.".into()),
                    changes: Vec::new(),
                    timestamp: None,
                    tokens: None,
                }],
                agents: Vec::new(),
                tokens: None,
            },
            summary: None,
        };
        commit.source_digest = commit.calculate_source_digest();
        AissenceDocument {
            version: DOCUMENT_VERSION,
            project: aissence::AissenceProject {
                source_id: Uuid::from_u128(1),
                name: Some("test-project".into()),
                repository: None,
            },
            commits: vec![commit],
            ignored: BTreeSet::new(),
        }
    }

    #[test]
    fn validates_hashes_and_digests() {
        assert!(valid_commit_hash(&"a".repeat(40)));
        assert!(!valid_commit_hash(&"A".repeat(40)));
        assert!(valid_digest(&format!("sha256:{}", "0".repeat(64))));
        assert!(!valid_digest("sha256:short"));
    }

    #[test]
    fn source_only_sync_does_not_require_a_remote_summary() {
        assert!(stored_commit_satisfies_upload(true, false, false, 0, 0));
        assert!(!stored_commit_satisfies_upload(
            true,
            false,
            false,
            aissence::SUMMARIZER_VERSION,
            0,
        ));
        assert!(stored_commit_satisfies_upload(
            true,
            false,
            false,
            aissence::SUMMARIZER_VERSION,
            aissence::SUMMARIZER_VERSION as i32,
        ));
    }

    #[test]
    fn slug_is_normalized() {
        assert_eq!(normalized_name(Some("  Demo  ")), "Demo");
    }

    #[test]
    fn upload_parser_rejects_malformed_json_before_database_work() {
        assert!(matches!(
            parse_upload(&test_config(), b"not-json"),
            Err(ApiError::Malformed(_))
        ));
    }

    #[test]
    fn upload_parser_accepts_a_clean_strict_v2_document() {
        let body = serde_json::to_vec(&sample_document()).unwrap();
        assert!(parse_upload(&test_config(), &body).is_ok());
    }

    #[test]
    fn upload_parser_rejects_unsanitized_secrets() {
        let mut document = sample_document();
        document.commits[0].title = "api_key=secret-value-123456789".into();
        document.commits[0].source_digest = document.commits[0].calculate_source_digest();
        let body = serde_json::to_vec(&document).unwrap();
        assert!(matches!(
            parse_upload(&test_config(), &body),
            Err(ApiError::Invalid(_))
        ));
    }

    #[test]
    fn upload_parser_rejects_empty_prompt_summaries_before_database_work() {
        let mut document = sample_document();
        document.commits[0].summary = Some(aissence::CommitSummary {
            summarizer_version: aissence::SUMMARIZER_VERSION,
            public: aissence::PublicStory {
                subline: String::new(),
                summary: "The documentation was updated.".into(),
                beats: Vec::new(),
                origin: aissence::SummaryOrigin::Model,
            },
            messages: vec![aissence::MessageSummary {
                kind: MessageKind::UserWrote,
                subscriber_summary: Some(" \n ".into()),
                content: Some("Please update the documentation.".into()),
                changes: Vec::new(),
                timestamp: None,
                tokens: None,
            }],
            agents: Vec::new(),
            tokens: None,
            prompt_failures: Vec::new(),
        });
        let body = serde_json::to_vec(&document).unwrap();

        assert!(matches!(
            parse_upload(&test_config(), &body),
            Err(ApiError::Invalid(detail)) if detail.contains("empty prompt summary")
        ));
    }

    #[test]
    fn upload_parser_repairs_legacy_empty_summaries_for_empty_prompts() {
        let mut document = sample_document();
        document.commits[0].source.messages[0].content = Some(" \n ".into());
        document.commits[0].source_digest = document.commits[0].calculate_source_digest();
        document.commits[0].summary = Some(aissence::CommitSummary {
            summarizer_version: aissence::SUMMARIZER_VERSION,
            public: aissence::PublicStory {
                subline: String::new(),
                summary: "An empty request entry was recorded.".into(),
                beats: Vec::new(),
                origin: aissence::SummaryOrigin::Model,
            },
            messages: vec![aissence::MessageSummary {
                kind: MessageKind::UserWrote,
                subscriber_summary: Some(String::new()),
                content: Some(" \n ".into()),
                changes: Vec::new(),
                timestamp: None,
                tokens: None,
            }],
            agents: Vec::new(),
            tokens: None,
            prompt_failures: Vec::new(),
        });
        let body = serde_json::to_vec(&document).unwrap();

        let parsed = parse_upload(&test_config(), &body).unwrap();
        assert_eq!(
            parsed.commits[0].summary.as_ref().unwrap().messages[0]
                .subscriber_summary
                .as_deref(),
            Some(aissence::summarize::EMPTY_USER_SUMMARY)
        );
    }

    #[test]
    fn upload_parser_accepts_partial_prompt_results_with_retry_metadata() {
        let mut document = sample_document();
        document.commits[0].summary = Some(aissence::CommitSummary {
            summarizer_version: aissence::SUMMARIZER_VERSION,
            public: aissence::PublicStory {
                subline: String::new(),
                summary: "The documentation behavior was updated safely.".into(),
                beats: Vec::new(),
                origin: aissence::SummaryOrigin::Model,
            },
            messages: document.commits[0].source.messages.clone(),
            agents: Vec::new(),
            tokens: None,
            prompt_failures: vec![aissence::PromptSummaryFailure {
                message_index: 0,
                model: "nvidia/example".into(),
                error: "user-summary validation failed: JSON shape".into(),
                attempted_at: chrono::DateTime::UNIX_EPOCH,
            }],
        });
        let body = serde_json::to_vec(&document).unwrap();

        let parsed = parse_upload(&test_config(), &body).unwrap();
        assert_eq!(
            parsed.commits[0].summary.as_ref().unwrap().prompt_failures[0].message_index,
            0
        );
    }

    #[test]
    fn upload_parser_accepts_legacy_generated_sublines() {
        let mut document = sample_document();
        document.commits[0].summary = Some(aissence::CommitSummary {
            summarizer_version: aissence::SUMMARIZER_VERSION,
            public: aissence::PublicStory {
                subline: "Obsolete duplicate summary.".into(),
                summary: "The requested documentation behavior was clarified.".into(),
                beats: Vec::new(),
                origin: aissence::SummaryOrigin::Model,
            },
            messages: document.commits[0].source.messages.clone(),
            agents: Vec::new(),
            tokens: None,
            prompt_failures: Vec::new(),
        });

        let body = serde_json::to_vec(&document).unwrap();
        assert!(parse_upload(&test_config(), &body).is_ok());
    }

    #[tokio::test]
    async fn upload_limit_replaces_axums_two_megabyte_default() {
        const CONFIGURED_LIMIT: usize = 3 * 1024 * 1024;
        const ABOVE_AXUM_DEFAULT: usize = 2 * 1024 * 1024 + 1;
        let app = Router::new()
            .route("/", routing::post(|_body: Bytes| async {}))
            .layer(upload_body_limit(CONFIGURED_LIMIT));

        let accepted = app
            .clone()
            .oneshot(
                Request::post("/")
                    .body(Body::from(vec![0; ABOVE_AXUM_DEFAULT]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);

        let rejected = app
            .oneshot(
                Request::post("/")
                    .body(Body::from(vec![0; CONFIGURED_LIMIT + 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
