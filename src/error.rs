use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;
pub type AppResult<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Authorization error: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Upgrade required: {0}")]
    UpgradeRequired(String),

    /// 424 — the PROVIDER was reached and refused (or could not be reached).
    ///
    /// WHY 424 AND NOT 500: `GET /api/v1/integrations/coreswift/lists` mapped a rejected stored key
    /// onto [`AppError::Internal`], whose arm LOGS the reason and answers the caller a bare
    /// "Internal server error" — so a tenant could not tell a rejected key from a wrong base URL from
    /// a CoreSwift outage, and a 5xx says OUR server broke. 424 Failed Dependency carries the
    /// provider's own words, and being a 4xx it passes through Cloudflare verbatim (CF REPLACES an
    /// origin 502/504 body with its own page — measured 2026-10-02 on this fleet, MissedCall).
    #[error("Upstream refused: {0}")]
    UpstreamRefused(String),
    /// Received and understood, but nothing could be done with it (no side effect happened).
    #[error("Unprocessable: {0}")]
    UnprocessableEntity(String),

    /// Too many failed sign-ins from one address (see `crate::login_throttle`). **429**, with a
    /// `Retry-After` header and the wait spelled out in the sentence the panel shows.
    #[error("Too many sign-in attempts: retry after {retry_after_secs}s")]
    TooManyAttempts { retry_after_secs: u64 },
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Read the pause budget BEFORE the match moves `self`.
        let too_many_retry = match &self {
            AppError::TooManyAttempts { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        };
        let (status, error_message) = match self {
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            }
            AppError::Auth(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Jwt(e) => {
                tracing::error!("JWT error: {}", e);
                (StatusCode::UNAUTHORIZED, "Invalid token".to_string())
            }
            AppError::Reqwest(e) => {
                tracing::error!("HTTP client error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "External service error".to_string(),
                )
            }
            AppError::Io(e) => {
                tracing::error!("IO error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::NotImplemented(msg) => {
                tracing::warn!("Not implemented: {}", msg);
                (
                    StatusCode::NOT_IMPLEMENTED,
                    format!("Not implemented: {}", msg),
                )
            }
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::UpgradeRequired(msg) => (StatusCode::PAYMENT_REQUIRED, msg),
            AppError::UnprocessableEntity(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
            AppError::UpstreamRefused(msg) => (StatusCode::FAILED_DEPENDENCY, msg),
            // Too many failed sign-ins from one address — **429**, carrying the pause and (below)
            // a `Retry-After` header. Deliberately not a 401: a paused caller is not told whether
            // the credential was right, and a log reader can tell the two apart.
            AppError::TooManyAttempts { retry_after_secs } => (StatusCode::TOO_MANY_REQUESTS, {
                let minutes = (retry_after_secs.div_ceil(60)).max(1);
                format!(
                    "Too many failed sign-in attempts from this address. Try again in about \
                     {minutes} minute{}. If you have forgotten your password, use \
                     \"Forgot your password?\".",
                    if minutes == 1 { "" } else { "s" }
                )
            }),
        };

        // The paused-sign-in answer carries its retry budget in a header as well as in the body.
        if let Some(retry_after_secs) = too_many_retry {
            let body = Json(json!({
                "error": error_message,
                "status": StatusCode::TOO_MANY_REQUESTS.as_u16(),
                "retry_after_seconds": retry_after_secs,
            }));
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(
                    axum::http::header::RETRY_AFTER,
                    retry_after_secs.to_string(),
                )],
                body,
            )
                .into_response();
        }

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}
