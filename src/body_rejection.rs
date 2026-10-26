//! A malformed request body answers the app's own JSON — never axum's deserializer sentence.
//!
//! `src/error.rs::json_rejection` fixed this for the three doors that call it by hand
//! (`/auth/login`, `/auth/register`, `/auth/signup`). Every OTHER handler in this app still
//! takes a plain `Json<T>` extractor, and a bad body is refused by that extractor BEFORE the
//! handler runs — `text/plain`, e.g.:
//!
//! ```text
//! Failed to deserialize the JSON body into the target type: name: invalid type: integer `5`,
//!   expected a string at line 1 column 20
//! ```
//!
//! The console's fetch helper runs `await r.json()` before it checks `r.ok`, so on a plain-text
//! body it throws `Unexpected token 'F'` and the operator sees nothing at all — the same defect
//! [`crate::path_rejection`] closed for ids, on the body side.
//!
//! This is the structural half of that fix: one layer outside auth and outside every handler, so
//! a route added later inherits it. It is deliberately narrow — it only touches a `400/413/415/422`
//! whose body is `text/plain` AND begins with one of the framework's own framing prefixes, so a
//! handler-written `AppError` (already JSON) and the id rewrite next door both pass through byte
//! for byte. The sentence itself comes from [`crate::error::plain_english`], the same mapper the
//! three hand-written doors use, so the app has ONE vocabulary for this.

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

/// Statuses an extractor can produce for a body it cannot read.
const EXTRACTOR_STATUSES: [StatusCode; 4] = [
    StatusCode::BAD_REQUEST,
    StatusCode::UNSUPPORTED_MEDIA_TYPE,
    StatusCode::UNPROCESSABLE_ENTITY,
    StatusCode::PAYLOAD_TOO_LARGE,
];

/// Framing axum puts around the part the caller actually needs. A body that does not start with
/// one of these is NOT the extractor's refusal and is left untouched.
const EXTRACTOR_PREFIXES: [&str; 3] = [
    "Failed to deserialize the JSON body into the target type: ",
    "Failed to parse the request body as JSON: ",
    "Failed to buffer the request body: ",
];

/// A rejection body is one short sentence; this only bounds the buffer.
const MAX_REJECTION_BYTES: usize = 64 * 1024;

pub async fn body_rejection_is_app_json(request: Request, next: Next) -> Response {
    let response = next.run(request).await;

    if !EXTRACTOR_STATUSES.contains(&response.status()) {
        return response;
    }
    let is_plain_text = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/plain"))
        .unwrap_or(false);
    if !is_plain_text {
        return response;
    }

    let status = response.status();
    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_REJECTION_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return rewrite(
                parts,
                status,
                "The request body could not be read.".to_string(),
            )
        }
    };

    let raw = String::from_utf8_lossy(&bytes);
    let raw = raw.trim();
    let Some(unframed) = strip_framing(raw) else {
        // Not the extractor's sentence — put it back exactly as it arrived.
        return Response::from_parts(parts, Body::from(bytes));
    };

    let message = crate::error::plain_english(unframed);
    rewrite(parts, status, message)
}

/// Replace the body and content type, keeping every other header the inner layers set.
fn rewrite(
    mut parts: axum::http::response::Parts,
    status: StatusCode,
    message: String,
) -> Response {
    parts.status = status;
    let body = serde_json::json!({ "error": message, "status": status.as_u16() }).to_string();
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Ok(len) = HeaderValue::from_str(&body.len().to_string()) {
        parts.headers.insert(header::CONTENT_LENGTH, len);
    }
    Response::from_parts(parts, Body::from(body))
}

/// `None` when the body is not one of the framework's framed refusals.
fn strip_framing(raw: &str) -> Option<&str> {
    EXTRACTOR_PREFIXES
        .iter()
        .find_map(|prefix| raw.strip_prefix(prefix))
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extractors_framing_is_recognised_and_removed() {
        assert_eq!(
            strip_framing(
                "Failed to deserialize the JSON body into the target type: name: invalid type: \
                 integer `5`, expected a string at line 1 column 20"
            ),
            Some("name: invalid type: integer `5`, expected a string at line 1 column 20")
        );
        assert_eq!(
            strip_framing(
                "Failed to parse the request body as JSON: expected value at line 1 column 1"
            ),
            Some("expected value at line 1 column 1")
        );
    }

    #[test]
    fn a_body_that_is_not_the_extractors_is_out_of_scope() {
        // The id rewrite next door and every handler sentence must fall through untouched.
        assert_eq!(strip_framing("Invalid URL: UUID parsing failed"), None);
        assert_eq!(
            strip_framing(r#"{"error":"Invalid credentials","status":401}"#),
            None
        );
        assert_eq!(strip_framing(""), None);
    }
}
