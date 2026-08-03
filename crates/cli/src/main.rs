use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use aissence::remote::{
    AccessTokenResponse, ApiErrorBody, DeviceLoginStart, DeviceTokenRequest, DeviceTokenResponse,
    ImportAccepted, ImportStatus, LogoutRequest, ManifestCommit, ProjectState, RefreshTokenRequest,
    ResolveProjectRequest, ServiceConfiguration, SyncPlanRequest, SyncPlanResponse, WhoAmI,
};
use aissence::{
    AissenceDocument, LlmApi, LlmConfig, LlmConfigOverrides, Summarizer, SummaryStore, analyze,
    persist::read_document, persist::write_document,
};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

const DEFAULT_SERVER: &str = "http://127.0.0.1:8787";
const KEYRING_SERVICE: &str = "aissence";

#[derive(Parser)]
#[command(
    name = "aissence",
    version,
    about = "Analyze and share AI-assisted commit history"
)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authenticate this CLI through the browser.
    Login {
        #[arg(long, default_value = DEFAULT_SERVER)]
        server: String,
    },
    /// Remove locally stored server credentials.
    Logout {
        #[arg(long, default_value = DEFAULT_SERVER)]
        server: String,
    },
    /// Show the authenticated server user.
    AuthStatus {
        #[arg(long, default_value = DEFAULT_SERVER)]
        server: String,
    },
    /// Analyze a folder and write an unsummarized `.aissence` file.
    Analyze {
        #[arg(default_value = ".")]
        folder: PathBuf,
    },
    /// Summarize locally through an OpenAI-compatible API or through the server.
    Summarize {
        #[arg(default_value = ".")]
        folder: PathBuf,
        #[arg(long, value_enum, default_value_t = Mode::Local)]
        mode: Mode,
        #[arg(long, default_value = DEFAULT_SERVER)]
        server: String,
        #[command(flatten)]
        llm: LlmOptions,
    },
    /// Analyze and incrementally upload only missing or changed commits.
    Sync {
        #[arg(default_value = ".")]
        folder: PathBuf,
        #[arg(long, default_value = DEFAULT_SERVER)]
        server: String,
    },
    /// Manually upload an existing `.aissence` file.
    Upload {
        file: PathBuf,
        #[arg(long, default_value = DEFAULT_SERVER)]
        server: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Local,
    Server,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LlmApiArgument {
    Openrouter,
    OpenaiCompatible,
}

impl From<LlmApiArgument> for LlmApi {
    fn from(value: LlmApiArgument) -> Self {
        match value {
            LlmApiArgument::Openrouter => Self::OpenRouter,
            LlmApiArgument::OpenaiCompatible => Self::OpenaiCompatible,
        }
    }
}

#[derive(Default, clap::Args)]
struct LlmOptions {
    /// TOML config file; defaults to <folder>/.aissence.toml when present.
    #[arg(long)]
    llm_config: Option<PathBuf>,
    /// Request dialect used by the endpoint.
    #[arg(long, value_enum)]
    api: Option<LlmApiArgument>,
    /// API base URL including /v1 (the client appends /chat/completions).
    #[arg(long)]
    base_url: Option<String>,
    /// Bearer token. Prefer --api-key-env to avoid shell history exposure.
    #[arg(long)]
    api_key: Option<String>,
    /// Name of an environment variable containing the bearer token.
    #[arg(long)]
    api_key_env: Option<String>,
    /// Model identifier understood by the endpoint.
    #[arg(long)]
    model: Option<String>,
    /// Preferred OpenRouter provider (invalid for generic endpoints).
    #[arg(long)]
    provider: Option<String>,
}

impl LlmOptions {
    fn overrides(self) -> LlmConfigOverrides {
        LlmConfigOverrides {
            api: self.api.map(Into::into),
            base_url: self.base_url,
            api_key: self.api_key,
            api_key_env: self.api_key_env,
            model: self.model,
            provider: self.provider,
        }
    }

    fn is_empty(&self) -> bool {
        self.llm_config.is_none()
            && self.api.is_none()
            && self.base_url.is_none()
            && self.api_key.is_none()
            && self.api_key_env.is_none()
            && self.model.is_none()
            && self.provider.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}

struct UploadOutcome {
    merged: usize,
    preview_url: String,
    created: bool,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    match Arguments::parse().command {
        Command::Login { server } => login(&server).await,
        Command::Logout { server } => logout(&server).await,
        Command::AuthStatus { server } => auth_status(&server).await,
        Command::Analyze { folder } => analyze_to_file(&folder),
        Command::Summarize {
            folder,
            mode: Mode::Local,
            llm,
            ..
        } => summarize_local(&folder, llm).await,
        Command::Summarize {
            folder,
            mode: Mode::Server,
            server,
            llm,
            ..
        } => {
            if !llm.is_empty() {
                return Err("local LLM options cannot be used with --mode server".to_string());
            }
            sync_folder(&folder, &server).await
        }
        Command::Sync { folder, server } => sync_folder(&folder, &server).await,
        Command::Upload { file, server } => upload_file(&file, &server).await,
    }
}

fn analyze_to_file(folder: &Path) -> Result<(), String> {
    let history = analyze(folder).map_err(|error| error.to_string())?;
    let mut store = SummaryStore::load(folder);
    store
        .sync_groups(&history.groups)
        .map_err(|error| format!("cannot write {}: {error}", store.path().display()))?;
    println!(
        "Wrote {} with {} commit(s).",
        store.path().display(),
        store.len()
    );
    Ok(())
}

async fn summarize_local(folder: &Path, llm: LlmOptions) -> Result<(), String> {
    let mut history = analyze(folder).map_err(|error| error.to_string())?;
    let mut store = SummaryStore::load(folder);
    for group in &mut history.groups {
        if let Some(cached) = store.get_for_group(group)
            && aissence::summarize::has_required_public_story(group, cached)
        {
            group.summary = Some(cached.clone());
        }
    }
    let config_path = llm.llm_config.clone();
    let config = LlmConfig::load(folder, config_path.as_deref(), llm.overrides())
        .map_err(|error| error.to_string())?;
    let summarizer = Summarizer::from_config(config).map_err(|error| error.to_string())?;
    let (outcome, cost) = summarizer.summarize_history_with_cost(&mut history).await;
    outcome.map_err(|error| error.to_string())?;
    store
        .replace_from_groups(&history.groups)
        .map_err(|error| error.to_string())?;
    let unpriced = if cost.unpriced_request_count == 0 {
        String::new()
    } else {
        format!(", {} unpriced", cost.unpriced_request_count)
    };
    println!(
        "Wrote {} with local summaries ({} request(s), ${:.6}{}).",
        store.path().display(),
        cost.request_count,
        cost.usd,
        unpriced
    );
    Ok(())
}

async fn sync_folder(folder: &Path, server: &str) -> Result<(), String> {
    let history = analyze(folder).map_err(|error| error.to_string())?;
    let mut store = SummaryStore::load(folder);
    store
        .sync_groups(&history.groups)
        .map_err(|error| error.to_string())?;
    let mut document = store.document().clone();
    let outcome = upload_document(&mut document, server).await;
    write_document(store.path(), &document).map_err(|error| error.to_string())?;
    let outcome = outcome?;
    println!(
        "Merged {} server summary result(s) into {}.",
        outcome.merged,
        store.path().display()
    );
    print_ledger_link(&outcome);
    Ok(())
}

async fn upload_file(path: &Path, server: &str) -> Result<(), String> {
    let mut document = read_document(path).map_err(|error| error.to_string())?;
    let outcome = upload_document(&mut document, server).await;
    write_document(path, &document).map_err(|error| error.to_string())?;
    let outcome = outcome?;
    println!(
        "Uploaded {} and merged {} result(s).",
        path.display(),
        outcome.merged
    );
    print_ledger_link(&outcome);
    Ok(())
}

async fn upload_document(
    document: &mut AissenceDocument,
    server: &str,
) -> Result<UploadOutcome, String> {
    validate_server_url(server)?;
    aissence::Redactor::global().redact_document(document);
    let client = reqwest::Client::new();
    let mut tokens = load_or_login(&client, server).await?;
    let project: ProjectState = match authenticated_get_optional(
        &client,
        &format!(
            "{server}/v1/projects/by-source/{}",
            document.project.source_id
        ),
        &tokens.access_token,
    )
    .await?
    {
        Some(project) => project,
        None => {
            let (name, description) = prompt_project_metadata(document)?;
            document.project.name = Some(name.clone());
            send_json(
                client
                    .put(format!("{server}/v1/projects/resolve"))
                    .bearer_auth(&tokens.access_token),
                &ResolveProjectRequest {
                    source_id: document.project.source_id,
                    name: Some(name),
                    description: Some(description),
                    repository: document.project.repository.clone(),
                },
            )
            .await?
        }
    };
    let plan: SyncPlanResponse = send_json(
        client
            .post(format!("{server}/v1/projects/{}/sync-plan", project.id))
            .bearer_auth(&tokens.access_token),
        &SyncPlanRequest {
            file_version: document.version,
            commits: document
                .commits
                .iter()
                .map(|commit| ManifestCommit {
                    commit_hash: commit.commit_hash.clone(),
                    source_digest: commit.source_digest.clone(),
                })
                .collect(),
        },
    )
    .await?;
    let requested: BTreeSet<String> = plan
        .upload
        .into_iter()
        .map(|commit| commit.commit_hash)
        .collect();
    if requested.is_empty() {
        println!(
            "Server already has every commit at revision {}.",
            plan.project_revision
        );
        return Ok(UploadOutcome {
            merged: 0,
            preview_url: project.preview_url,
            created: project.created,
        });
    }
    let delta = document.delta(&requested);
    if delta.commits.len() != requested.len() {
        return Err("server requested a commit that is not present in the local document".into());
    }
    let response = client
        .post(format!("{server}/v1/projects/{}/imports", project.id))
        .bearer_auth(&tokens.access_token)
        .header("Content-Type", "application/vnd.aissence+json")
        .body(serde_json::to_vec(&delta).map_err(|error| error.to_string())?)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let accepted: ImportAccepted = decode_response(response).await?;
    println!(
        "Import {} accepted: {} queued, {} skipped.",
        accepted.import_id, accepted.queued_commit_count, accepted.skipped_commit_count
    );
    loop {
        refresh_if_needed(&client, server, &mut tokens).await?;
        let status: ImportStatus = authenticated_get(
            &client,
            &format!("{server}/v1/imports/{}", accepted.import_id),
            &tokens.access_token,
        )
        .await?;
        if status.status != "processing" {
            let result: AissenceDocument = authenticated_get(
                &client,
                &format!("{server}/v1/imports/{}/result", accepted.import_id),
                &tokens.access_token,
            )
            .await?;
            result.validate_version()?;
            if result.project.source_id != document.project.source_id {
                return Err("server returned summaries for a different project".into());
            }
            let merged = document.merge_summaries(&result);
            if status.status == "failed" {
                return Err(status
                    .error_detail
                    .unwrap_or_else(|| "remote summarization failed".into()));
            }
            return Ok(UploadOutcome {
                merged,
                preview_url: project.preview_url,
                created: project.created,
            });
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn print_ledger_link(outcome: &UploadOutcome) {
    let label = if outcome.created {
        "Created draft ledger"
    } else {
        "View your ledger"
    };
    println!("{label}: {}", outcome.preview_url);
}

async fn load_or_login(client: &reqwest::Client, server: &str) -> Result<StoredTokens, String> {
    let mut tokens = match load_tokens(server) {
        Ok(tokens) => tokens,
        Err(_) if io::stdin().is_terminal() => {
            println!("You are not logged in to {server}.");
            if !prompt_confirmation("Log in now?", true)? {
                return Err("login is required to create or sync a ledger".into());
            }
            login(server).await?;
            load_tokens(server)?
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = validate_login(client, server, &mut tokens).await {
        if !is_unauthorized_error(&error) || !io::stdin().is_terminal() {
            return Err(error);
        }
        println!("Your saved login is no longer valid for {server}.");
        if !prompt_confirmation("Log in again?", true)? {
            return Err("login is required to create or sync a ledger".into());
        }
        login(server).await?;
        tokens = load_tokens(server)?;
        validate_login(client, server, &mut tokens).await?;
    }
    Ok(tokens)
}

async fn validate_login(
    client: &reqwest::Client,
    server: &str,
    tokens: &mut StoredTokens,
) -> Result<(), String> {
    refresh_if_needed(client, server, tokens).await?;
    let _: WhoAmI =
        authenticated_get(client, &format!("{server}/v1/whoami"), &tokens.access_token).await?;
    Ok(())
}

fn is_unauthorized_error(error: &str) -> bool {
    error.contains("401 Unauthorized") || error.contains("unauthorized")
}

fn prompt_project_metadata(document: &AissenceDocument) -> Result<(String, String), String> {
    let default_name = document
        .project
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Aissence project");
    if !io::stdin().is_terminal() {
        return Ok((default_name.to_string(), String::new()));
    }

    println!("This folder does not have a remote ledger yet.");
    let name = loop {
        let name = prompt_value("Project name", default_name)?;
        let name = name.trim();
        if name.is_empty() {
            eprintln!("Project name cannot be empty.");
        } else if name.chars().count() > 120 {
            eprintln!("Project name must be at most 120 characters.");
        } else {
            break name.to_string();
        }
    };
    let description = loop {
        let description = prompt_value("Description (optional)", "")?;
        let description = description.trim();
        if description.chars().count() > 3_000 {
            eprintln!("Description must be at most 3000 characters.");
        } else {
            break description.to_string();
        }
    };
    Ok((name, description))
}

fn prompt_value(label: &str, default: &str) -> Result<String, String> {
    if default.is_empty() {
        print!("{label}: ");
    } else {
        print!("{label} [{default}]: ");
    }
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?;
    let value = value.trim_end_matches(['\r', '\n']);
    Ok(if value.is_empty() {
        default.to_string()
    } else {
        value.to_string()
    })
}

fn prompt_confirmation(label: &str, default: bool) -> Result<bool, String> {
    let hint = if default { "Y/n" } else { "y/N" };
    print!("{label} [{hint}] ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?;
    match value.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => Err("please answer yes or no".into()),
    }
}

async fn login(server: &str) -> Result<(), String> {
    validate_server_url(server)?;
    let client = reqwest::Client::new();
    let configuration: ServiceConfiguration =
        get_json(&client, &format!("{server}/v1/config")).await?;
    let start: DeviceLoginStart = decode_response(
        client
            .post(&configuration.login_start_endpoint)
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )
    .await?;
    validate_server_url(&start.verification_uri_complete)?;
    println!(
        "Open this URL to sign in:\n\n{}\n",
        start.verification_uri_complete
    );
    if let Err(error) = open::that(&start.verification_uri_complete) {
        eprintln!("Could not open the browser automatically: {error}");
    }
    let started = chrono::Utc::now().timestamp();
    let response = loop {
        let response: DeviceTokenResponse = send_json(
            client.post(format!("{server}/v1/auth/device/token")),
            &DeviceTokenRequest {
                request_id: start.request_id,
                device_code: start.device_code.clone(),
            },
        )
        .await?;
        match response.status.as_str() {
            "authorized" => break response,
            "pending" if chrono::Utc::now().timestamp() - started < start.expires_in as i64 => {
                tokio::time::sleep(Duration::from_secs(start.interval)).await;
            }
            "expired" => return Err("browser login expired; run login again".into()),
            status => return Err(format!("browser login failed with status {status}")),
        }
    };
    let tokens = StoredTokens {
        access_token: response
            .access_token
            .ok_or("login response omitted access token")?,
        refresh_token: response
            .refresh_token
            .ok_or("login response omitted refresh token")?,
        expires_at: chrono::Utc::now().timestamp()
            + response.expires_in.ok_or("login response omitted expiry")?,
    };
    save_tokens(server, &tokens)?;
    println!("Login complete; credentials are stored in the OS credential store.");
    Ok(())
}

async fn logout(server: &str) -> Result<(), String> {
    validate_server_url(server)?;
    let entry = credential_entry(server)?;
    if let Ok(value) = entry.get_password()
        && let Ok(tokens) = serde_json::from_str::<StoredTokens>(&value)
    {
        let response = reqwest::Client::new()
            .post(format!("{server}/v1/auth/logout"))
            .json(&LogoutRequest {
                refresh_token: tokens.refresh_token,
            })
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() && response.status() != reqwest::StatusCode::UNAUTHORIZED
        {
            let error: ApiErrorBody = response.json().await.map_err(|error| error.to_string())?;
            return Err(error.detail.unwrap_or(error.error));
        }
    }
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(error.to_string()),
    }
    println!("Removed local Aissence credentials.");
    Ok(())
}

async fn auth_status(server: &str) -> Result<(), String> {
    validate_server_url(server)?;
    let client = reqwest::Client::new();
    let mut tokens = load_tokens(server)?;
    refresh_if_needed(&client, server, &mut tokens).await?;
    let user: WhoAmI = authenticated_get(
        &client,
        &format!("{server}/v1/whoami"),
        &tokens.access_token,
    )
    .await?;
    println!("Authenticated as {}.", user.user_id);
    Ok(())
}

async fn refresh_if_needed(
    client: &reqwest::Client,
    server: &str,
    tokens: &mut StoredTokens,
) -> Result<(), String> {
    if tokens.expires_at > chrono::Utc::now().timestamp() + 60 {
        return Ok(());
    }
    let response = client
        .post(format!("{server}/v1/auth/refresh"))
        .json(&RefreshTokenRequest {
            refresh_token: tokens.refresh_token.clone(),
        })
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let response: AccessTokenResponse = decode_response(response).await?;
    tokens.access_token = response.access_token;
    tokens.refresh_token = response.refresh_token;
    tokens.expires_at = chrono::Utc::now().timestamp() + response.expires_in;
    save_tokens(server, tokens)
}

fn credential_entry(server: &str) -> Result<keyring::Entry, String> {
    let identifier = format!(
        "server-{}",
        hex::encode(Sha256::digest(server.trim_end_matches('/').as_bytes()))
    );
    keyring::Entry::new(KEYRING_SERVICE, &identifier).map_err(|error| error.to_string())
}

fn validate_server_url(value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|error| format!("invalid server URL: {error}"))?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() == "https" || (local && url.scheme() == "http") {
        Ok(())
    } else {
        Err("server URLs must use HTTPS outside localhost".into())
    }
}

fn save_tokens(server: &str, tokens: &StoredTokens) -> Result<(), String> {
    credential_entry(server)?
        .set_password(&serde_json::to_string(tokens).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn load_tokens(server: &str) -> Result<StoredTokens, String> {
    let value = credential_entry(server)?
        .get_password()
        .map_err(|_| "not logged in; run `aissence login` first".to_string())?;
    serde_json::from_str(&value).map_err(|error| format!("stored credentials are invalid: {error}"))
}

async fn get_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T, String> {
    decode_response(
        client
            .get(url)
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )
    .await
}

async fn authenticated_get<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<T, String> {
    decode_response(
        client
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )
    .await
}

async fn authenticated_get_optional<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<Option<T>, String> {
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(None)
    } else {
        decode_response(response).await.map(Some)
    }
}

async fn send_json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
    body: &impl Serialize,
) -> Result<T, String> {
    decode_response(
        request
            .json(body)
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )
    .await
}

async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let detail = serde_json::from_slice::<ApiErrorBody>(&body)
            .ok()
            .and_then(|body| body.detail.or(Some(body.error)))
            .unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
        return Err(format!("server returned {status}: {detail}"));
    }
    serde_json::from_slice(&body).map_err(|error| format!("invalid server response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_credentials_only_travel_over_https_or_localhost() {
        assert!(validate_server_url("https://aissence.example/v1").is_ok());
        assert!(validate_server_url("http://127.0.0.1:8787").is_ok());
        assert!(validate_server_url("http://localhost:8787").is_ok());
        assert!(validate_server_url("http://aissence.example").is_err());
        assert!(validate_server_url("file:///tmp/server").is_err());
    }
}
