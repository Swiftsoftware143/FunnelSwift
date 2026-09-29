//! `/api/v1/internal/sync-plan-tag` — sibling-service → FunnelSwift plan/tag sync.
//!
//! **History (why this file looks like this).** This handler used to be 13 lines that took
//! `State(_state)` and `Json(_payload)` — ignoring both — and answered
//! `{"synced": true, "message": "Plan-tag sync triggered"}` unconditionally. It checked no
//! credential and touched no database, so an anonymous empty POST returned 200 and ADASwift —
//! which calls this on every plan create/delete and logs the response as success — believed
//! plan→tag mappings were syncing. They never were: `plan_tag_mappings` held 0 rows.
//!
//! **What it does now.** Fails closed on a missing or wrong internal key, and answers honestly
//! instead of fabricating a success it did not perform.
//!
//! **Why it cannot simply "do the sync".** The real writer,
//! `plan_tag_handler::sync_plan_tag_mappings` (`POST /api/v1/plan-tag-mappings/sync`), requires
//! `plan_tag_mappings.plan_id` AND `.tag_id`, both **NOT NULL foreign keys** — to `plans(id)` and
//! `tags(id)` respectively. ADASwift sends only `action` + `plan_name` (`"Pro"`), and FunnelSwift's
//! own six plans are Agency/Scale, Capture Free, Capture Starter, Kinetic Free, Kinetic Pro and
//! Suite. There is **no rule that maps one to the other, and no tag to attach.** Inventing one would
//! write a wrong tag against a wrong plan — worse than the no-op it replaced, because it would then
//! look real. So this refuses, loudly and truthfully, until the mapping rule is defined.
//!
//! When the rule exists, implement it here by mirroring `plan_tag_handler::sync_plan_tag_mappings`
//! (DELETE by plan_id, then one INSERT per tag) rather than by re-adding a second writer.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::HeaderMap, Json};
use serde_json::Value;

/// Constant-time comparison, matching the sibling internal handlers
/// (`cross_app_webhook_handler::ct_eq`, `portfolio_sync_handler::ct_eq`), so the key check does not
/// leak a prefix or length through timing.
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Receives a sibling's plan create/delete notification.
///
/// Accepted credential forms:
///   * `X-Internal-Key: <key>`  — the fleet standard, used by every other internal handler.
///   * `api_key` in the JSON body — **ADASwift's current shape**, honoured so a caller is not
///     silently locked out mid-migration. Moving ADASwift to the header is follow-up work in
///     ADASwift's own lane; do not remove body support before that lands.
///
/// Never logs or echoes the expected key.
pub async fn sync_plan_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let header_key = headers
        .get("x-internal-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let body_key = payload
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Fail closed. An unset server key must refuse rather than accept anything.
    let authorised = !state.internal_sync_key.is_empty()
        && (ct_eq(header_key, &state.internal_sync_key)
            || ct_eq(body_key, &state.internal_sync_key));
    if !authorised {
        return Err(AppError::Unauthorized("Invalid internal key".into()));
    }

    let action = payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let plan_name = payload
        .get("plan_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source_app = payload
        .get("source_app")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Authenticated, but unable to do the work for a real reason. Say so.
    Err(AppError::NotImplemented(format!(
        "sync-plan-tag is not implemented: no mapping rule exists from {source_app} plan \
         {plan_name:?} (action {action:?}) to a FunnelSwift plan_id + tag_id. \
         plan_tag_mappings requires both (NOT NULL FKs) and is currently empty, so no mapping \
         can be written. Define the mapping rule, or have an admin use \
         POST /api/v1/plan-tag-mappings/sync with an explicit plan_id and tag_ids."
    )))
}

#[cfg(test)]
mod tests {
    use super::ct_eq;

    /// The check must accept an exact match and reject everything else, including the two cases
    /// that would silently pass a naive `starts_with`/`contains` implementation.
    #[test]
    fn ct_eq_is_exact_and_length_sensitive() {
        assert!(ct_eq("abc123", "abc123"));
        assert!(!ct_eq("abc123", "abc124"), "one differing byte must fail");
        assert!(!ct_eq("abc123", "abc"), "a prefix must not pass");
        assert!(!ct_eq("abc", "abc123"), "a longer value must not pass");
        assert!(!ct_eq("", "abc"), "empty must not pass a real key");
        assert!(ct_eq("", ""), "empty==empty is the caller's job to reject");
    }
}
