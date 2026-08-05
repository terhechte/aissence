use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use aissence::remote::ApiErrorBody;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("authentication required")]
    Unauthorized,
    #[error("resource not found")]
    NotFound,
    #[error("request body is too large")]
    PayloadTooLarge,
    #[error("unsupported content type")]
    UnsupportedMediaType,
    #[error("malformed JSON: {0}")]
    Malformed(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("service limit reached: {0}")]
    Limit(String),
    #[error("database error: {0}")]
    Database(#[from] tokio_postgres::Error),
    #[error("database pool error: {0}")]
    Pool(#[from] deadpool_postgres::PoolError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, detail) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", None),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", None),
            Self::PayloadTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", None),
            Self::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                None,
            ),
            Self::Malformed(detail) => (StatusCode::BAD_REQUEST, "malformed_json", Some(detail)),
            Self::Invalid(detail) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_document",
                Some(detail),
            ),
            Self::Conflict(detail) => (StatusCode::CONFLICT, "conflict", Some(detail)),
            Self::Limit(detail) => (StatusCode::TOO_MANY_REQUESTS, "limit_reached", Some(detail)),
            error @ (Self::Database(_) | Self::Pool(_) | Self::Internal(_)) => {
                tracing::error!(error = ?error, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", None)
            }
        };
        (
            status,
            Json(ApiErrorBody {
                error: code.to_string(),
                detail,
            }),
        )
            .into_response()
    }
}
