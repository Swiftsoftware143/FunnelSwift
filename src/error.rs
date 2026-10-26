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

/// The app's own answer for a request body axum could not read as JSON.
///
/// Measured LIVE 2026-10-26 (cron `default`): the PUBLIC doors answered with axum's own
/// Rust sentence in a shape no client of this app reads —
/// `POST /api/v1/auth/signup` with a non-JSON body → `400 "Failed to parse the request body
/// as JSON: expected value at line 1 column 1"`, and `POST /api/v1/auth/register` with no
/// password → `422 "Failed to deserialize the JSON body into the target type: missing field
/// `password` at line 1 column 71"`. Every other refusal from these handlers is
/// `{"error": …, "status": …}`, and the marketing signup page parses that shape: handed the
/// raw sentence it shows the customer a message about "the target type" and a byte offset.
///
/// Handlers take the extractor's own `Result` and pass the rejection through here, so the
/// refusal names the offending FIELD (which is useful) in the app's shape, and drops the
/// Rust prefix and the line/column tail (which are not).
pub(crate) fn json_rejection(rej: axum::extract::rejection::JsonRejection) -> AppError {
    use axum::extract::rejection::JsonRejection;
    // Syntax/no-content-type are the caller's framing errors: say that in one plain sentence.
    // A DATA error is worth naming the field for, because the caller can fix it from the message.
    match &rej {
        JsonRejection::MissingJsonContentType(_) => {
            return AppError::BadRequest(
                "Send this request as JSON (Content-Type: application/json).".to_string(),
            )
        }
        JsonRejection::JsonSyntaxError(_) => {
            return AppError::BadRequest("The request body is not valid JSON.".to_string())
        }
        _ => {}
    }
    let raw = rej.body_text();
    let msg = raw
        .trim_start_matches("Failed to deserialize the JSON body into the target type: ")
        .trim_start_matches("Failed to parse the request body as JSON: ");
    let msg = msg.split(" at line ").next().unwrap_or(msg).trim();
    AppError::BadRequest(plain_english(msg))
}

/// Turn the deserializer's words into a sentence a customer can act on.
///
/// Stripping the framing and the byte offset was not enough: the remainder is still the JSON
/// parser's own grammar, and the marketing signup page shows it verbatim. Measured LIVE
/// 2026-10-26 — `POST /api/v1/auth/login` with `{"email": 12345, "password": 99}` answered
/// `The request body was not accepted: email: invalid type: integer `12345`, expected a string`,
/// and an empty `{}` answered `The request body was not accepted: missing field `email``. A
/// person reading that learns the name of our Rust type, not what to do. Every sibling app in
/// the fleet now answers the plain sentence below; this closes the last one.
pub(crate) fn plain_english(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();

    if lower.contains("length limit") {
        return "That upload is too large. Choose a smaller file.".to_string();
    }
    if lower.contains("content-type") || lower.contains("unsupported media") {
        return "Send this request as JSON (Content-Type: application/json).".to_string();
    }

    if let Some(rest) = lower.split("missing field").nth(1) {
        let field = rest
            .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("")
            .to_string();
        return match field.as_str() {
            "email" => "Enter your email address.".to_string(),
            "password" => "Enter your password.".to_string(),
            "name" => "Enter your name.".to_string(),
            "new_password" => "Choose a new password.".to_string(),
            "token" | "code" => "Paste the code from your email.".to_string(),
            "" => "A required field is missing from this form.".to_string(),
            other => format!("The form is missing {other}."),
        };
    }

    if lower.contains("invalid type")
        || lower.contains("invalid value")
        || lower.contains("invalid length")
    {
        let shape = lower.contains("expected struct")
            || lower.starts_with("invalid type: null")
            || lower.contains("invalid length");
        return match leading_field(raw) {
            Some(field) => format!("{field}: that value is not valid for this form."),
            None if shape => "Send the form's fields as a JSON object.".to_string(),
            None => "One of the values in this form is not valid.".to_string(),
        };
    }

    if let Some(field) = leading_field(raw) {
        return format!("{field}: that value is not valid for this form.");
    }

    "The request body could not be read.".to_string()
}

/// The deserializer prefixes a field-scoped complaint with `field: `. Only take a token that looks
/// like a field name, so an unrecognised sentence never becomes `Failed to: that value is not…`.
fn leading_field(raw: &str) -> Option<String> {
    let (head, _) = raw.split_once(':')?;
    let head = head.trim();
    let looks_like_field = !head.is_empty()
        && head.len() <= 40
        && head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    looks_like_field.then(|| head.replace('_', " "))
}
