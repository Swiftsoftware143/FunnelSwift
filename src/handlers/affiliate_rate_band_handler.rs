use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};

/// POST /api/v1/internal/affiliate/apply-rate-bands
///
/// Recompute every active affiliate's standing rate from the performance bands.
///
/// David, 2026-10-03: *"maybe if they have a certain amount of customers I will increase their
/// percentage ... or if they have customers that have multiple apps then they can earn a higher
/// percentage. Is there a way to automate that?"*
///
/// This is the automation. It is meant to be called on a schedule (a deterministic job, not an agent
/// turn). It only ever WRITES the affiliate's standing `affiliates.commission_rate` (already one of the
/// six sources `commission.rs` resolves a rate through) plus the explainability columns
/// (`rate_band_id` / `rate_reason` / `rate_updated_at`). So escalation changes a stored number rather
/// than adding a seventh source that every commission calculation would have to reason about.
///
/// NOT RETROACTIVE, BY CONSTRUCTION: no commission is recomputed or rewritten here. A rate recorded
/// before this runs stays exactly as it was.
///
/// THE TWO SIGNALS, both already recorded by the app:
///   * paying customers — `affiliate_commissions` joined to `leads`; a customer counts once they have
///     earned the affiliate a commission and that commission has not been reversed. The MONEY RECORD is
///     the authority here, deliberately: inferring "paying" from a plan name would be a guess that drifts
///     the moment plan names change.
///   * apps per customer — `DISTINCT tags.source_app` on those customers' tenants. `tags.source_app` is
///     already how a sibling app's plan is identified (the code's own example is "ADASwift — Free"), so
///     multi-app usage needs no new instrumentation.
///
/// Idempotent: running it twice changes nothing the second time, which is what lets it run on a timer.
pub async fn apply_rate_bands(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> AppResult<Json<Value>> {
    let key = headers
        .get("x-internal-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if state.internal_sync_key.is_empty() || key != state.internal_sync_key {
        return Err(AppError::Unauthorized("Invalid internal key".into()));
    }

    // Best rate first: the first band whose conditions are met wins.
    let bands: Vec<(uuid::Uuid, String, i32, i32, f64)> = sqlx::query_as(
        "SELECT id, label, min_paying_customers, min_apps_per_customer, rate::float8
           FROM affiliate_rate_bands
          WHERE is_active = true
          ORDER BY rate DESC, min_paying_customers DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("load bands: {e}")))?;

    let affiliates: Vec<(String, uuid::Uuid, Option<f64>)> = sqlx::query_as(
        "SELECT id, user_id, commission_rate::float8 FROM affiliates WHERE is_active = true",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("load affiliates: {e}")))?;

    let (mut changed, mut unchanged, mut skipped, mut relabelled) = (0, 0, 0, 0);
    let mut moved: Vec<Value> = Vec::new();

    for (affiliate_id, user_id, current) in affiliates {
        let paying: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT l.tenant_id)
               FROM affiliate_commissions c
               JOIN leads l ON l.id = c.lead_id
              WHERE c.affiliate_id = $1 AND c.reversed_at IS NULL",
        )
        .bind(&affiliate_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("count customers for {affiliate_id}: {e}")))?;

        let apps: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT t.source_app)
               FROM leads l
               JOIN tags t ON t.tenant_id = l.tenant_id
              WHERE l.created_by = $1
                AND t.is_system = true
                AND t.source_app IS NOT NULL
                AND t.source_app <> ''",
        )
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("count apps for {affiliate_id}: {e}")))?;

        // A live affiliate is at least on one app, even before any system tag exists for them: every band
        // requires 1 app, so a 0 here would make every band unreachable for a brand-new affiliate.
        let paying_i = paying as i32;
        let apps_i = std::cmp::max(apps as i32, 1);

        let Some((band_id, label, _, _, rate)) = bands
            .iter()
            .find(|(_, _, min_cust, min_apps, _)| paying_i >= *min_cust && apps_i >= *min_apps)
        else {
            skipped += 1;
            continue;
        };

        let reason =
            format!("{label} — {paying_i} paying customer(s), {apps_i} app(s) per customer");

        if current.map(|c| (c - rate).abs() < 0.005).unwrap_or(false) {
            // The RATE is already correct, but the explanation may be missing (a fresh affiliate is
            // seeded straight onto its plan rate, so its first evaluation reads "unchanged") or stale
            // (a band's label or thresholds were edited). Refresh only the explainability columns —
            // never the rate — so the affiliate's own console can always answer "why this rate?".
            // Still idempotent: the guarded UPDATE matches nothing once the stored reason/band agree.
            let refreshed = sqlx::query(
                "UPDATE affiliates
                    SET rate_band_id = $2, rate_reason = $3, rate_updated_at = now()
                  WHERE id = $1
                    AND (rate_band_id IS DISTINCT FROM $2 OR rate_reason IS DISTINCT FROM $3)",
            )
            .bind(&affiliate_id)
            .bind(band_id)
            .bind(&reason)
            .execute(&state.pool)
            .await
            .map_err(|e| AppError::Internal(format!("refresh reason for {affiliate_id}: {e}")))?
            .rows_affected();
            if refreshed > 0 {
                relabelled += 1;
            } else {
                unchanged += 1;
            }
            continue;
        }

        sqlx::query(
            "UPDATE affiliates
                SET commission_rate = $2::numeric,
                    rate_band_id = $3,
                    rate_reason = $4,
                    rate_updated_at = now()
              WHERE id = $1",
        )
        .bind(&affiliate_id)
        .bind(rate)
        .bind(band_id)
        .bind(&reason)
        .execute(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("update rate for {affiliate_id}: {e}")))?;

        changed += 1;
        moved.push(json!({
            "affiliate_id": affiliate_id,
            "from": current,
            "to": rate,
            "band": label,
            "reason": reason,
        }));
    }

    Ok(Json(json!({
        "changed": changed,
        "unchanged": unchanged,
        "skipped": skipped,
        "relabelled": relabelled,
        "moved": moved,
    })))
}
