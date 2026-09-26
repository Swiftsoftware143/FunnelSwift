//! Cross-app conversion receiver.
//!
//! `POST /api/v1/webhooks/conversion` is posted by the fleet's sibling apps when a paid
//! checkout carries referral/affiliate data (WorkflowSwift, ADASwift and missedcallrespondr
//! all point `FUNNELSWIFT_URL` at this app). Until kanban t_f408b7cc this module was a
//! two-function no-op: it answered `200 {"received": true}` and wrote nothing, so the
//! referring affiliate was never credited and the senders — which read the 2xx — never
//! retried.
//!
//! It now writes the row the attribution path reads: `affiliate_commissions`, the ledger the
//! affiliate portal sums (`affiliate_portal_handler`), the affiliate's conversion list
//! (`affiliate_handler`) and the payout handler pay out from. Attribution is the fleet link
//! `leads.created_by -> affiliates.user_id` (same resolution as
//! `/api/v1/internal/affiliate/upgrade-event`), with a direct `affiliate_id` fallback.
//!
//! **A 2xx always means a row was written.** An authorised caller whose conversion cannot be
//! attributed gets 422 and no row, never a receipt for work that did not happen.
//!
//! The sibling `POST /api/v1/track/lead` was deleted by the same card: it had no caller in
//! the fleet and no consumer.
//!
//! Auth: the route is in `global_auth::PUBLIC_EXACT` because the callers send no JWT, so the
//! handler gates itself on `x-internal-key` exactly like this app's other internal receivers
//! (`/api/v1/internal/affiliate/upgrade-event`, `/api/v1/internal/portfolio-sync`).

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::HeaderMap, Json};
use serde_json::{json, Value};
use uuid::Uuid;

/// Length-independent constant-time compare of the internal key.
fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

/// POST /api/v1/webhooks/conversion
///
/// Body keys (all optional except `source_app`): `source_app`, `event`, `affiliate_id`,
/// `cookie_id`, `lead_id`, `lead_email` (or `email`), `product_id`, `product_name`,
/// `amount`, `provider_session_id`, `event_id`, `metadata`.
pub async fn handle_conversion_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let key = headers
        .get("x-internal-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if state.internal_sync_key.is_empty() || !ct_eq(key, &state.internal_sync_key) {
        return Err(AppError::Unauthorized("Invalid internal key".into()));
    }

    let source_app = payload["source_app"].as_str().unwrap_or("");
    if source_app.is_empty() {
        return Err(AppError::BadRequest("source_app is required".into()));
    }
    let event = payload["event"].as_str().unwrap_or("conversion");
    let amount = payload["amount"].as_f64().unwrap_or(0.0);
    let cookie_id = payload["cookie_id"].as_str().map(str::to_string);
    let provider_session_id = payload["provider_session_id"].as_str().map(str::to_string);
    let lead_email = payload["lead_email"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| payload["email"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_string);

    // Idempotency: the senders retry a purchase, so crediting twice must be impossible.
    // `event_id` (the fleet's upgrade-event key) or `provider_session_id` (the checkout
    // session) identifies the purchase.
    if let Some(k) = payload["event_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(provider_session_id.as_deref())
    {
        let already: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM affiliate_commissions
             WHERE metadata->>'event_id' = $1 OR metadata->>'provider_session_id' = $1
             LIMIT 1",
        )
        .bind(k)
        .fetch_optional(&state.pool)
        .await?;
        if let Some(id) = already {
            return Ok(Json(json!({
                "status": "already-recorded",
                "id": id.to_string(),
            })));
        }
    }

    // ── Attribution ──────────────────────────────────────────────────────────────────────
    // The permanent fleet link is leads.created_by -> affiliates.user_id (the affiliate who
    // captured the lead, whether or not the referral cookie still exists).
    let mut lead_id: Option<Uuid> = payload["lead_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok());
    let mut created_by: Option<Uuid> = None;
    if let Some(id) = lead_id {
        created_by = sqlx::query_scalar("SELECT created_by FROM leads WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    } else if let Some(ref email) = lead_email {
        let row: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
            "SELECT id, created_by FROM leads WHERE email = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(email)
        .fetch_optional(&state.pool)
        .await?;
        if let Some((id, by)) = row {
            lead_id = Some(id);
            created_by = by;
        }
    }

    let mut affiliate: Option<(String, Option<f64>)> = None;
    if let Some(user_id) = created_by {
        affiliate = sqlx::query_as(
            "SELECT id, commission_rate::float8 FROM affiliates
             WHERE user_id = $1 AND is_active = true LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?;
    }
    if affiliate.is_none() {
        // Direct fallback: the caller sends the affiliate id it captured from the referral
        // link. Honoured only if that affiliate exists here — an unknown id must not reach
        // the NOT NULL / FK on affiliate_commissions.
        if let Some(aid) = payload["affiliate_id"].as_str().filter(|s| !s.is_empty()) {
            affiliate = sqlx::query_as(
                "SELECT id, commission_rate::float8 FROM affiliates
                 WHERE id = $1 AND is_active = true LIMIT 1",
            )
            .bind(aid)
            .fetch_optional(&state.pool)
            .await?;
        }
    }
    let Some((affiliate_id, rate)) = affiliate else {
        // Nothing was written, so this cannot be a 2xx: a false receipt is what this card is
        // about. The senders are fire-and-forget, so the log line is their only trace.
        tracing::warn!(
            "cross-app conversion NOT recorded (unattributed): source_app={} lead_id={:?} lead_email={:?} affiliate_id={:?}",
            source_app,
            lead_id,
            lead_email,
            payload["affiliate_id"].as_str()
        );
        return Err(AppError::UnprocessableEntity(
            "conversion not attributable: no lead or affiliate matches lead_id, lead_email or affiliate_id"
                .into(),
        ));
    };

    // Product: the caller's product_id when this DB knows it, else the product registered
    // for that source app.
    let mut product_id: Option<Uuid> = payload["product_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok());
    if let Some(pid) = product_id {
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM affiliate_products WHERE id = $1)")
                .bind(pid)
                .fetch_one(&state.pool)
                .await?;
        if !known {
            product_id = None;
        }
    }
    if product_id.is_none() {
        product_id = sqlx::query_scalar(
            "SELECT id FROM affiliate_products WHERE source_app = $1 AND is_active = true LIMIT 1",
        )
        .bind(source_app)
        .fetch_optional(&state.pool)
        .await?;
    }

    let commission = amount * rate.unwrap_or(0.0) / 100.0;
    let metadata = json!({
        "source_app": source_app,
        "event": event,
        "event_id": payload["event_id"].as_str(),
        "provider_session_id": provider_session_id,
        "cookie_id": cookie_id,
        "product_name": payload["product_name"].as_str(),
        "sale_amount": amount,
    });

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO affiliate_commissions (id, affiliate_id, lead_id, product_id, amount, status, metadata)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6)",
    )
    .bind(id)
    .bind(&affiliate_id)
    .bind(lead_id)
    .bind(product_id)
    .bind(commission)
    .bind(&metadata)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        "cross-app conversion recorded: source_app={} event={} affiliate={} amount={} commission={}",
        source_app,
        event,
        affiliate_id,
        amount,
        commission
    );

    Ok(Json(json!({
        "status": "recorded",
        "id": id.to_string(),
        "affiliate_id": affiliate_id,
        "lead_id": lead_id.map(|v| v.to_string()),
        "product_id": product_id.map(|v| v.to_string()),
        "sale_amount": amount,
        "commission": commission,
    })))
}
