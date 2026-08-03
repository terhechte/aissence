use std::net::SocketAddr;

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    pub supabase_jwt_secret: String,
    pub token_secret: String,
    pub token_issuer: String,
    pub token_audience: String,
    pub public_url: String,
    pub site_url: String,
    pub access_token_seconds: i64,
    pub refresh_token_days: i64,
    pub supabase_auth_url: String,
    pub supabase_anon_key: String,
    pub max_upload_bytes: usize,
    pub max_commits_per_upload: usize,
    pub max_messages_per_commit: usize,
    pub max_entry_chars: usize,
    pub daily_user_request_limit: i64,
    pub daily_global_request_limit: i64,
    pub daily_user_input_chars: i64,
    pub daily_global_input_chars: i64,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind = value("AISSENCE_BIND", "127.0.0.1:8787")
            .parse()
            .map_err(|error| format!("invalid AISSENCE_BIND: {error}"))?;
        let auth_url = value("SUPABASE_AUTH_URL", "http://127.0.0.1:55421/auth/v1");
        let config = Self {
            bind,
            database_url: value(
                "DATABASE_URL",
                "postgresql://postgres:postgres@127.0.0.1:55422/postgres",
            ),
            supabase_jwt_secret: value(
                "SUPABASE_JWT_SECRET",
                "super-secret-jwt-token-with-at-least-32-characters-long",
            ),
            token_secret: value(
                "AISSENCE_TOKEN_SECRET",
                "local-aissence-token-secret-change-in-production-32chars",
            ),
            token_issuer: value("AISSENCE_TOKEN_ISSUER", "aissence-server"),
            token_audience: value("AISSENCE_TOKEN_AUDIENCE", "aissence-cli"),
            public_url: value("AISSENCE_PUBLIC_URL", "http://127.0.0.1:8787")
                .trim_end_matches('/')
                .to_string(),
            site_url: value("AISSENCE_SITE_URL", "http://127.0.0.1:5173")
                .trim_end_matches('/')
                .to_string(),
            access_token_seconds: number("AISSENCE_ACCESS_TOKEN_SECONDS", 900)?,
            refresh_token_days: number("AISSENCE_REFRESH_TOKEN_DAYS", 30)?,
            supabase_auth_url: auth_url.trim_end_matches('/').to_string(),
            supabase_anon_key: value("SUPABASE_ANON_KEY", ""),
            max_upload_bytes: number("AISSENCE_MAX_UPLOAD_BYTES", 32 * 1024 * 1024)?,
            max_commits_per_upload: number("AISSENCE_MAX_COMMITS", 2_000)?,
            max_messages_per_commit: number("AISSENCE_MAX_MESSAGES_PER_COMMIT", 2_000)?,
            max_entry_chars: number("AISSENCE_MAX_ENTRY_CHARS", 64 * 1024)?,
            daily_user_request_limit: number("AISSENCE_DAILY_REQUEST_LIMIT", 2_000)?,
            daily_global_request_limit: number("AISSENCE_DAILY_GLOBAL_REQUEST_LIMIT", 10_000)?,
            daily_user_input_chars: number("AISSENCE_DAILY_USER_INPUT_CHARS", 5_000_000)?,
            daily_global_input_chars: number("AISSENCE_DAILY_GLOBAL_INPUT_CHARS", 50_000_000)?,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.token_secret.len() < 32 || self.supabase_jwt_secret.len() < 32 {
            return Err("JWT secrets must each contain at least 32 bytes".into());
        }
        if !(60..=86_400).contains(&self.access_token_seconds) {
            return Err("AISSENCE_ACCESS_TOKEN_SECONDS must be between 60 and 86400".into());
        }
        if !(1..=365).contains(&self.refresh_token_days) {
            return Err("AISSENCE_REFRESH_TOKEN_DAYS must be between 1 and 365".into());
        }
        if self.max_upload_bytes == 0
            || self.max_commits_per_upload == 0
            || self.max_messages_per_commit == 0
            || self.max_entry_chars == 0
            || self.daily_user_request_limit <= 0
            || self.daily_global_request_limit <= 0
            || self.daily_user_input_chars <= 0
            || self.daily_global_input_chars <= 0
        {
            return Err("server safety limits must be positive".into());
        }
        let public_url = reqwest::Url::parse(&self.public_url)
            .map_err(|error| format!("invalid AISSENCE_PUBLIC_URL: {error}"))?;
        let local = matches!(
            public_url.host_str(),
            Some("127.0.0.1" | "localhost" | "::1")
        );
        if !local && public_url.scheme() != "https" {
            return Err("AISSENCE_PUBLIC_URL must use HTTPS outside localhost".into());
        }
        let auth_url = reqwest::Url::parse(&self.supabase_auth_url)
            .map_err(|error| format!("invalid SUPABASE_AUTH_URL: {error}"))?;
        let local_auth = matches!(auth_url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
        if !local_auth && auth_url.scheme() != "https" {
            return Err("SUPABASE_AUTH_URL must use HTTPS outside localhost".into());
        }
        let site_url = reqwest::Url::parse(&self.site_url)
            .map_err(|error| format!("invalid AISSENCE_SITE_URL: {error}"))?;
        let local_site = matches!(site_url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
        if !local_site && site_url.scheme() != "https" {
            return Err("AISSENCE_SITE_URL must use HTTPS outside localhost".into());
        }
        if !local
            && (std::env::var("AISSENCE_TOKEN_SECRET").is_err()
                || std::env::var("SUPABASE_JWT_SECRET").is_err()
                || std::env::var("AISSENCE_SITE_URL").is_err())
        {
            return Err(
                "production JWT secrets and AISSENCE_SITE_URL must be explicitly configured".into(),
            );
        }
        Ok(())
    }
}

fn value(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn number<T>(name: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr + Copy,
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|error| format!("invalid {name}: {error}")),
        Err(_) => Ok(default),
    }
}
