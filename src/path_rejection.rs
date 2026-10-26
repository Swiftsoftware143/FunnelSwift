//! Path-parameter rejections answer the app's own JSON 404 — never axum's parser sentence.
//!
//! `Path<Uuid>` is used by ~70 handlers in this app. When a caller hands one a stale, truncated
//! or hand-typed id, axum's extractor fails before the handler runs and its rejection is rendered
//! as `400 text/plain`:
//!
//! ```text
//! Invalid URL: UUID parsing failed: invalid character: found `n` at 0
//! ```
//!
//! That sentence is the framework talking to a browser: it names no resource, it is not JSON, and
//! the console's `await r.json()` on it throws `Unexpected token 'I'`. Measured live 2026-10-26 on
//! this app: nine authenticated id reads still answered that way (`/funnels/{id}`, `/leads/{id}`,
//! `/tags/{id}`, `/plans/{id}`, `/tenants/{id}`, `/admin/plans/{id}`, `/admin/email-templates/{id}`,
//! `/card-analytics/{card_id}`, `/portfolio-companies/{id}`) while `/campaigns/{id}`,
//! `/affiliates/{id}` and `/play/{id}` already answered a clean JSON 404.
//!
//! Rewriting the ~70 handler signatures one by one would be churn with no extra coverage, and any
//! handler added later would silently reintroduce the leak. This layer fixes the class in one
//! place: it sits outside auth and outside every handler, so it sees whatever the extractors emit.
//!
//! It is deliberately narrow — it only touches a `400` whose body is `text/plain` AND begins with
//! axum's `Invalid URL` marker, and it re-emits every other response byte for byte. A malformed id
//! then gets the same answer as an id with no row, which is what a stale console link should say.

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The first bytes of every axum `Path`/`Query` rejection, and of nothing else this app emits.
const REJECTION_MARKER: &[u8] = b"Invalid URL";

/// A rejection body is one short sentence; this only bounds the buffer.
const MAX_REJECTION_BYTES: usize = 64 * 1024;

pub async fn path_rejection_is_not_found(request: Request, next: Next) -> Response {
    let response = next.run(request).await;

    // Every JSON error this app writes is `application/json`; axum's extractor rejections are the
    // only `text/plain` bodies on the API surface. Checking this first keeps the common path cheap
    // and leaves binary/streaming responses alone without reading them.
    let is_plain_text = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/plain"))
        .unwrap_or(false);
    if response.status() != StatusCode::BAD_REQUEST || !is_plain_text {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_REJECTION_BYTES).await {
        Ok(b) => b,
        // Cannot judge it, so do not pretend: a 400 we could not read stays a 400.
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    match not_found_for(&bytes) {
        Some(replacement) => replacement,
        // Not the framework's sentence — put the response back exactly as it arrived.
        None => Response::from_parts(parts, Body::from(bytes)),
    }
}

/// The whole decision, with no I/O: does this body carry axum's parser sentence?
fn not_found_for(bytes: &[u8]) -> Option<Response> {
    if !bytes.starts_with(REJECTION_MARKER) {
        return None;
    }
    Some(
        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": "Not found",
                "status": 404,
            })),
        )
            .into_response(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes as read_body;

    const LEAKED: &[u8] = b"Invalid URL: UUID parsing failed: invalid character: found `n` at 0";

    #[test]
    fn the_parser_sentence_becomes_a_json_404() {
        let out = not_found_for(LEAKED).expect("the parser sentence must be replaced");
        assert_eq!(out.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            out.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[test]
    fn an_application_400_is_not_touched() {
        assert!(not_found_for(br#"{"error":"Name is required","status":400}"#).is_none());
    }

    #[test]
    fn other_plain_text_is_not_touched() {
        assert!(not_found_for(b"some other plain text refusal").is_none());
    }

    #[test]
    fn a_json_success_body_is_not_touched() {
        assert!(not_found_for(br#"{"id":"abc"}"#).is_none());
    }

    #[tokio::test]
    async fn the_replacement_body_is_json_a_console_can_parse() {
        let out = not_found_for(LEAKED).unwrap();
        let (_, body) = out.into_parts();
        let text = String::from_utf8(read_body(body, 4096).await.unwrap().to_vec()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["status"], 404);
        assert!(!text.contains("Invalid URL"), "{text}");
    }
}
