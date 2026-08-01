//! Stable HTTP payloads shared by the CLI and Aissence server.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfiguration {
    pub login_start_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceLoginStart {
    pub request_id: Uuid,
    pub device_code: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceTokenRequest {
    pub request_id: Uuid,
    pub device_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceTokenResponse {
    pub status: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveProjectRequest {
    pub source_id: Uuid,
    pub name: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectState {
    pub id: Uuid,
    pub sync_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestCommit {
    pub commit_hash: String,
    pub source_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncPlanRequest {
    pub file_version: u32,
    pub commits: Vec<ManifestCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadReason {
    Missing,
    Changed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedCommit {
    pub commit_hash: String,
    pub reason: UploadReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncPlanResponse {
    pub upload: Vec<RequestedCommit>,
    pub unchanged_count: usize,
    pub remote_only: Vec<String>,
    pub project_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportAccepted {
    pub import_id: Uuid,
    pub status: String,
    pub submitted_commit_count: usize,
    pub queued_commit_count: usize,
    pub skipped_commit_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportStatus {
    pub import_id: Uuid,
    pub status: String,
    pub submitted_commit_count: i32,
    pub summarized_commit_count: i32,
    pub skipped_commit_count: i32,
    pub failed_commit_count: i64,
    pub error_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhoAmI {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiErrorBody {
    pub error: String,
    pub detail: Option<String>,
}
