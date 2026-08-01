use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct SupabaseUser {
    pub id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct AissenceClaims {
    sub: String,
    exp: usize,
    iat: usize,
    iss: String,
    aud: String,
    role: String,
    jti: String,
}

#[derive(Debug, Deserialize)]
struct SupabaseClaims {
    sub: String,
    exp: usize,
    aud: serde_json::Value,
    role: Option<String>,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts)?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[state.config.token_audience.as_str()]);
        validation.set_issuer(&[state.config.token_issuer.as_str()]);
        let claims = decode::<AissenceClaims>(
            token,
            &DecodingKey::from_secret(state.config.token_secret.as_bytes()),
            &validation,
        )
        .map_err(|_| ApiError::Unauthorized)?
        .claims;
        if claims.role != "authenticated" {
            return Err(ApiError::Unauthorized);
        }
        let id = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
        ensure_active_user(state, id).await?;
        Ok(Self { id })
    }
}

impl FromRequestParts<AppState> for SupabaseUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts)?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&["authenticated"]);
        let claims = decode::<SupabaseClaims>(
            token,
            &DecodingKey::from_secret(state.config.supabase_jwt_secret.as_bytes()),
            &validation,
        )
        .map_err(|_| ApiError::Unauthorized)?
        .claims;
        let _ = (claims.exp, claims.aud);
        if claims.role.as_deref() != Some("authenticated") {
            return Err(ApiError::Unauthorized);
        }
        let id = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
        ensure_active_user(state, id).await?;
        Ok(Self { id })
    }
}

fn bearer(parts: &Parts) -> Result<&str, ApiError> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.trim().is_empty())
        .ok_or(ApiError::Unauthorized)
}

async fn ensure_active_user(state: &AppState, id: Uuid) -> Result<(), ApiError> {
    let client = state.pool.get().await?;
    let active = client
        .query_opt(
            "select 1 from auth.users where id = $1 and deleted_at is null \
             and (banned_until is null or banned_until < now()) and not is_anonymous",
            &[&id],
        )
        .await?
        .is_some();
    if active {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

pub fn issue_access_token(state: &AppState, user_id: Uuid) -> Result<String, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let claims = AissenceClaims {
        sub: user_id.to_string(),
        exp: (now + state.config.access_token_seconds) as usize,
        iat: now as usize,
        iss: state.config.token_issuer.clone(),
        aud: state.config.token_audience.clone(),
        role: "authenticated".into(),
        jti: Uuid::new_v4().to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(state.config.token_secret.as_bytes()),
    )
    .map_err(|error| ApiError::Internal(format!("cannot sign access token: {error}")))
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
