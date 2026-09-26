//! Request-body read deadline (kanban t_488a19d4).
//!
//! FunnelSwift is the lead-capture end of the fleet: a stranger can reach
//! `POST /k/:slug/lead`, `POST /api/v1/web-to-lead`, `POST /api/v1/auth/login`,
//! `POST /api/v1/auth/register`, `POST /api/v1/kinetic/cards/:id/gate` and the
//! `x-internal-key`-gated `/api/v1/internal/*` push routes — and every one of them reads a request
//! body through a `Json` extractor, which runs *before* the handler (so the key/signature/credential
//! check inside the handler happens *after* the body has arrived). Before this module a client could
//! send a request head with a valid `Content-Length` and then send nothing: the task, the connection
//! and the partially-read body buffer stayed pinned for ever — no 408, no close, no log line.
//! WorkflowSwift closed the same hole in t_e7cba83e and ADASwift in t_b3d626ed (408 at t+30.0 s);
//! this is the FunnelSwift arm of that fleet-wide bound.
//!
//! ## Which requests are bounded (the per-route decision, kanban t_488a19d4)
//!
//! `api_router::create_router` is ONE flat chain of 181 routes behind a single global
//! `require_auth` layer (`auth/global_auth.rs`), which keeps its own public/internal allowlist —
//! there is no public-vs-protected router split to hang a route-scoped layer on, and carving the
//! 750-line chain into two routers to gain nothing observable would risk silently dropping a route
//! off a live CRM-facing app. So the scope is expressed at the layer itself, by *method and declared
//! body*: a request is bounded iff it carries a body at all (`Content-Length > 0`, or
//! `Transfer-Encoding` present) and its method can carry one (`POST`/`PUT`/`PATCH`/`DELETE`).
//!
//! That gate is exactly the set of requests the defect is about, and it is a no-op on everything
//! else: every GET — including a GET that declares a body, and every route whose handler reads no
//! body at all — is handed to the inner service **untouched, with its original body**, and is
//! answered at t+0.0 s (proven live: a stalled body on `GET /api/health` answers at t+0.0 s on both
//! the pre- and post-change binary). A `POST` with `Content-Length: 0` is likewise untouched.
//!
//! ## Mount point
//!
//! Mounted INNERMOST inside `require_auth` (in `api_router::create_router` the layer is added
//! *before* the `require_auth` layer — the first layer added is the innermost). An unauthenticated
//! request with a declared-but-absent body is therefore answered `401` at t+0.00 s and never starts
//! waiting for a body, and a declared body is never buffered before the credential is checked.

use axum::{
    body::{Body, Bytes},
    extract::{FromRequest, Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tracing::warn;

/// Default body-read deadline: how long the body of a request that carries one may take to arrive
/// before the request is answered `408` and its task, connection and partially-read body buffer are
/// released. Override with `BODY_READ_DEADLINE_SECS`, clamped to `5..=300` by
/// [`clamp_body_read_deadline_secs`], so neither a typo nor a fat finger can shed real traffic.
///
/// 30 s is orders of magnitude above the time a real body on these routes takes — a lead capture or
/// a login is kilobytes over a same-region link — and still generous to a slow sender: a full 2 MiB
/// body may arrive as slowly as ~70 KiB/s and complete inside it.
pub const DEFAULT_BODY_READ_DEADLINE_SECS: u64 = 30;

/// The clamp applied to `BODY_READ_DEADLINE_SECS`. Pure so it is testable and so the number the
/// middleware enforces is the number the boot log printed.
pub fn clamp_body_read_deadline_secs(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_BODY_READ_DEADLINE_SECS)
        .clamp(5, 300)
}

/// How long a request body may take to arrive, as configured.
///
/// Its own type so the middleware can be mounted (and unit-tested) without an `AppState` — and so
/// the number it enforces is the number the boot log printed, never re-derived per request.
#[derive(Clone, Copy, Debug)]
pub struct BodyReadDeadline(std::time::Duration);

impl BodyReadDeadline {
    /// From the configured seconds. Clamping lives in [`clamp_body_read_deadline_secs`]; this is a
    /// plain carrier.
    pub fn from_secs(seconds: u64) -> Self {
        Self(std::time::Duration::from_secs(seconds))
    }

    /// From `BODY_READ_DEADLINE_SECS` (default [`DEFAULT_BODY_READ_DEADLINE_SECS`], clamped).
    pub fn from_env() -> Self {
        Self::from_secs(clamp_body_read_deadline_secs(
            std::env::var("BODY_READ_DEADLINE_SECS").ok().as_deref(),
        ))
    }

    /// The deadline as a duration.
    pub fn duration(self) -> std::time::Duration {
        self.0
    }
}

/// Does this request declare a body at all? A request with neither a positive `Content-Length` nor
/// a `Transfer-Encoding` carries nothing to wait for, so the deadline must not be started for it.
fn declares_a_body(parts: &axum::http::request::Parts) -> bool {
    if parts.headers.contains_key(header::TRANSFER_ENCODING) {
        return true;
    }
    parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .is_some_and(|len| len > 0)
}

/// Can this method carry a request body? The methods whose handlers on this app read one.
fn carries_a_body(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// The `408` a request whose body never finished arriving is answered with.
///
/// Carries the deadline so a client log says which bound was hit, and `Connection: close` because
/// the declared body was never consumed: this connection cannot be reused for another request.
fn body_deadline_response(deadline: std::time::Duration) -> Response {
    Response::builder()
        .status(StatusCode::REQUEST_TIMEOUT)
        .header("Connection", "close")
        .body(Body::from(format!(
            "{{\"error\":\"Request body was not received in time. Retry.\",\"status\":408,\
             \"body_deadline_seconds\":{}}}",
            deadline.as_secs()
        )))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::REQUEST_TIMEOUT)
                .header("Connection", "close")
                .body(Body::empty())
                .unwrap_or_else(|_| Response::new(Body::empty()))
        })
}

/// Middleware: bound how long a request BODY may take to arrive, and answer `408` if it has not
/// finished within [`BodyReadDeadline`] of the headers.
///
/// **Why a body deadline and not a handler budget.** The routes this covers do their own credential,
/// key or signature work *after* the body has been read: `/api/v1/internal/*` checks
/// `x-internal-key` inside the handler, `/api/v1/auth/*` hashes a password, `/k/:slug/lead` resolves
/// the card. A wall-clock budget over those routes would have to be sized for the handler's own
/// work, and elapsing it drops work that was legitimately in progress — for a captured lead or a
/// cross-app push that is a LOST EVENT, not a retried login. This middleware bounds only the thing
/// that is actually unbounded: a client that sends a request head and then stops. A handler that
/// legitimately takes seconds is untouched, because the deadline is over by the time it runs.
///
/// **The deadline is total, measured from the headers** — not a per-chunk idle timeout that resets
/// on every frame (`tower_http::timeout::RequestBodyTimeoutLayer` works that way). A resetting
/// timeout is not a bound here at all: a client that dribbles one byte every N-1 seconds holds the
/// task, the connection and the buffer for ever. On elapse the inner future is dropped, so the
/// partially-read body goes with it, the request is answered `408`, and the release is logged.
///
/// **Scope.** Only a request that declares a body and whose method can carry one is bounded (see the
/// module docs). Everything else — every GET, an empty POST — is passed to the inner service with
/// its original body, untouched and unread, and answers at t+0.0 s.
///
/// On success the bytes read here are handed to the inner service as an already-complete body, so
/// the handler's own extractor (`Json`, `Bytes`) sees exactly the bytes the client sent — same
/// bytes, same headers, same limit, because the read below *is* the same extractor: a body over
/// axum's `DefaultBodyLimit` is rejected through the identical code path, with the identical `413`.
pub async fn body_read_deadline_middleware(
    State(deadline): State<BodyReadDeadline>,
    request: Request,
    next: Next,
) -> Response {
    let deadline = deadline.duration();

    // The head is cloned rather than rebuilt so the inner request keeps the method, uri, version,
    // headers and every extension an outer layer inserted. Cloning `Parts` copies the extensions
    // map (shared handles), so nothing inserted by a layer above is lost.
    let (parts, body) = request.into_parts();

    if !(carries_a_body(&parts.method) && declares_a_body(&parts)) {
        // No body to wait for: unchanged behaviour, unchanged body, answered immediately.
        return next.run(Request::from_parts(parts, body)).await;
    }

    let probe = Request::from_parts(parts.clone(), body);

    // `()` is the extractor state: `Bytes::from_request` takes its limit from the request's own
    // extensions (axum's `DefaultBodyLimit`), not from the state, so the limit the handler would
    // have applied is the limit applied here.
    match tokio::time::timeout(deadline, Bytes::from_request(probe, &())).await {
        Ok(Ok(bytes)) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        // Over the body limit, or a read error on the way in: exactly the rejection the handler's
        // own extractor would have produced, produced by the same extractor.
        Ok(Err(rejection)) => rejection.into_response(),
        Err(_elapsed) => {
            warn!(
                "request body not received within {:?} (stalled body) — answered 408 for {} {}",
                deadline, parts.method, parts.uri
            );
            body_deadline_response(deadline)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use axum::routing::{get, post};
    use axum::Router;
    use tower::ServiceExt;

    // ── body-read deadline (kanban t_488a19d4) ────────────────────────────────────────────────
    //
    // Four properties a later refactor must not break: the deadline fires on a request that
    // declares a body and then stops; a body that does arrive reaches the handler byte for byte
    // (the `Json` extractors in the handlers parse these bytes, so anything else silently breaks
    // lead capture); the middleware does not widen the limit on how much one request may buffer;
    // and a request with nothing to wait for is untouched.

    /// A request body that produces no frame and never ends: the client-side shape of the hold
    /// this middleware bounds (head sent, body never arrives).
    struct StalledBody;

    impl futures_core::Stream for StalledBody {
        type Item = Result<Bytes, std::io::Error>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Pending
        }
    }

    /// The production mount shape: a route whose handler reads the body, next to a GET route whose
    /// handler reads none, both wrapped by the layer.
    fn body_deadline_app(deadline: std::time::Duration) -> Router {
        Router::new()
            .route("/webhook", post(|body: Bytes| async move { body }))
            .route("/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                BodyReadDeadline(deadline),
                body_read_deadline_middleware,
            ))
    }

    /// A stalled body on a route that reads one must end the hold: 408, `Connection: close`,
    /// promptly. Before this middleware the same request held a task, a connection and a
    /// partially-read body buffer for ever.
    #[tokio::test]
    async fn stalled_body_is_answered_408_within_the_deadline() {
        // 150 ms, not the configured 30 s: the assertion is about WHICH requests the deadline
        // fires on, not how long the operator set it to.
        let app = body_deadline_app(std::time::Duration::from_millis(150));
        let started = std::time::Instant::now();
        let resp = app
            .oneshot(
                HttpRequest::post("/webhook")
                    .header("content-length", "100000")
                    .body(Body::from_stream(StalledBody))
                    .expect("request"),
            )
            .await
            .expect("router");
        let elapsed = started.elapsed();

        assert_eq!(
            resp.status(),
            StatusCode::REQUEST_TIMEOUT,
            "a body that never arrives must be answered, not parked"
        );
        assert_eq!(
            resp.headers()
                .get("connection")
                .and_then(|v| v.to_str().ok()),
            Some("close"),
            "the declared body was never consumed, so the connection must not be reused"
        );
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("\"status\":408") && text.contains("\"body_deadline_seconds\""),
            "body was {}",
            text
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "the deadline must fire promptly, took {elapsed:?}"
        );
    }

    /// The bytes the client sends are the bytes the handler sees. This is why the deadline reads
    /// the body instead of replacing it: the handlers' `Json` extractors parse exactly these bytes.
    #[tokio::test]
    async fn complete_body_reaches_the_handler_unchanged() {
        let app = body_deadline_app(std::time::Duration::from_secs(30));
        let payload = br#"{"public_key":"pk_probe","email":"probe@example.test"}"#;
        let resp = app
            .oneshot(
                HttpRequest::post("/webhook")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_vec()))
                    .expect("request"),
            )
            .await
            .expect("router");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .expect("body");
        assert_eq!(
            &body[..],
            &payload[..],
            "the handler must see the raw request bytes, unmodified"
        );
    }

    /// A body over axum's own 2 MiB limit gets the extractor's own rejection: the deadline must not
    /// become a wider hole for a single unauthenticated request to pin memory.
    #[tokio::test]
    async fn body_over_the_default_limit_is_rejected_413() {
        let app = body_deadline_app(std::time::Duration::from_secs(30));
        let oversized = vec![b'x'; 2 * 1024 * 1024 + 1];
        let resp = app
            .oneshot(
                HttpRequest::post("/webhook")
                    .body(Body::from(oversized))
                    .expect("request"),
            )
            .await
            .expect("router");
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// The scope proof at unit level: a request with nothing to wait for is NOT held. A GET that
    /// declares a body and then stalls is handed to the handler untouched and answers immediately
    /// (a global server timeout, or a layer that read every body, would hold it for the deadline).
    /// An empty POST is the same case with a body-declaring method.
    #[tokio::test]
    async fn a_request_with_nothing_to_wait_for_is_not_held() {
        let app = body_deadline_app(std::time::Duration::from_millis(300));

        let started = std::time::Instant::now();
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::get("/health")
                    .header("content-length", "100000")
                    .body(Body::from_stream(StalledBody))
                    .expect("request"),
            )
            .await
            .expect("router");
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(150),
            "a GET must not wait for a declared body, took {:?}",
            started.elapsed()
        );

        let resp = app
            .oneshot(
                HttpRequest::post("/webhook")
                    .header("content-length", "0")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "an empty POST carries nothing to wait for and must be answered, not 408"
        );
    }

    /// The configured number is the number enforced: default, override, and the clamp that stops a
    /// typo from shedding real traffic.
    #[test]
    fn the_configured_bound_is_clamped() {
        assert_eq!(clamp_body_read_deadline_secs(None), 30);
        assert_eq!(clamp_body_read_deadline_secs(Some("45")), 45);
        assert_eq!(clamp_body_read_deadline_secs(Some(" 45 ")), 45);
        assert_eq!(clamp_body_read_deadline_secs(Some("0")), 5);
        assert_eq!(clamp_body_read_deadline_secs(Some("1")), 5);
        assert_eq!(clamp_body_read_deadline_secs(Some("86400")), 300);
        assert_eq!(clamp_body_read_deadline_secs(Some("abc")), 30);
        assert_eq!(clamp_body_read_deadline_secs(Some("-5")), 30);
    }
}
