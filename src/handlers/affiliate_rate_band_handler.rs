use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
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
///   * apps per customer — the `source_app` each EARNING commission recorded, grouped per referred
///     customer and read as the most apps any one of them holds. NOT read from `tags`: the `source_app`
///     tags live only under the System tenant, so a join through a lead's own tenant finds none and the
///     multi-app rule could never fire (measured 2026-10-03 on live).
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

    let affiliates: Vec<(String, Option<f64>)> =
        sqlx::query_as("SELECT id, commission_rate::float8 FROM affiliates WHERE is_active = true")
            .fetch_all(&state.pool)
            .await
            .map_err(|e| AppError::Internal(format!("load affiliates: {e}")))?;

    let (mut changed, mut unchanged, mut skipped, mut relabelled) = (0, 0, 0, 0);
    let mut moved: Vec<Value> = Vec::new();

    for (affiliate_id, current) in affiliates {
        // Both signals come from the money record, per referred customer.
        //
        // A "customer" is one referred lead that has actually EARNED a commission (status earned/paid,
        // not reversed) — the same definition `affiliate_portal_handler` uses for money already earned.
        // Customers are counted by the address the referral was captured under (the key
        // `cross_app_webhook_handler` resolves a lead by), falling back to the lead id when the capture
        // had no email.
        //
        // "apps per customer" is the most apps any ONE of those customers holds, taken from the
        // `source_app` each earning commission recorded. It is NOT read from `tags`: the `source_app`
        // tags are seeded only under the System tenant, so a join through a lead's OWN tenant resolves
        // zero rows for every real affiliate (every lead a capture form creates lives in the AFFILIATE's
        // tenant) and the multi-app rule could never fire — measured 2026-10-03 on live.
        let (paying, apps): (i64, i64) = sqlx::query_as(
            "SELECT count(DISTINCT s.cust),
                    COALESCE(max(s.apps), 0)
               FROM (
                    SELECT lower(coalesce(nullif(l.email, ''), l.id::text)) AS cust,
                           count(DISTINCT c.metadata->>'source_app') AS apps
                      FROM affiliate_commissions c
                      JOIN leads l ON l.id = c.lead_id
                     WHERE c.affiliate_id = $1
                       AND c.reversed_at IS NULL
                       AND c.status IN ('earned', 'paid')
                     GROUP BY 1
               ) s",
        )
        .bind(&affiliate_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("measure affiliate {affiliate_id}: {e}")))?;

        // A live affiliate is at least on one app, even before any earning carries a `source_app`:
        // every band requires 1 app, so a 0 here would make every band unreachable for a brand-new
        // affiliate.
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

/// ── Band CRUD (admin) ───────────────────────────────────────────────────────────────────────────
///
/// David, 2026-10-03: migration 094 seeds the bands as "STARTING VALUES for David to edit in the
/// panel", and the admin guide already tells the operator the bands "are rows you can edit". Until
/// these handlers existed there was no panel for that, so the guide was false. This is the missing
/// half: the seeded table becomes editable policy, without a code change per rate.
///
/// Admin-gated like every other affiliate write in this app (`auth.is_admin`). Deleting a band is
/// safe for money: `affiliates.rate_band_id` is `ON DELETE SET NULL`, so an affiliate keeps the
/// `commission_rate` the band had set — it only loses the label that explained it, and the next
/// recompute re-labels it.
#[derive(Debug, serde::Deserialize)]
pub struct BandInput {
    pub label: Option<String>,
    pub min_paying_customers: Option<i32>,
    pub min_apps_per_customer: Option<i32>,
    pub rate: Option<f64>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

fn require_admin(auth: &AuthUser) -> AppResult<()> {
    if auth.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin access required".into()))
    }
}

fn validate_rate(rate: f64) -> AppResult<()> {
    if !rate.is_finite() || !(0.0..=100.0).contains(&rate) {
        return Err(AppError::Validation(
            "rate must be a number between 0 and 100".into(),
        ));
    }
    Ok(())
}

fn clean_label(input: &Option<String>) -> Option<String> {
    input
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// GET /api/v1/affiliate-rate-bands — every band, ordered the way the operator reads them.
pub async fn list_bands(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    let rows: Vec<(
        uuid::Uuid,
        String,
        i32,
        i32,
        f64,
        i32,
        bool,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, label, min_paying_customers, min_apps_per_customer, rate::float8,
                sort_order, is_active, created_at, updated_at
           FROM affiliate_rate_bands
          ORDER BY sort_order, rate",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("list bands: {e}")))?;

    let bands: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                label,
                min_cust,
                min_apps,
                rate,
                sort_order,
                is_active,
                created_at,
                updated_at,
            )| {
                json!({
                    "id": id,
                    "label": label,
                    "min_paying_customers": min_cust,
                    "min_apps_per_customer": min_apps,
                    "rate": rate,
                    "sort_order": sort_order,
                    "is_active": is_active,
                    "created_at": created_at,
                    "updated_at": updated_at,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "bands": bands })))
}

/// POST /api/v1/affiliate-rate-bands — add a band.
pub async fn create_band(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(input): Json<BandInput>,
) -> AppResult<(StatusCode, Json<Value>)> {
    require_admin(&auth)?;
    let label = clean_label(&input.label)
        .ok_or_else(|| AppError::Validation("label is required".into()))?;
    let rate = input
        .rate
        .ok_or_else(|| AppError::Validation("rate is required".into()))?;
    validate_rate(rate)?;
    let min_cust = input.min_paying_customers.unwrap_or(0).max(0);
    // A band requires at least one app (migration 094): accepting 0 would make the app signal moot.
    let min_apps = input.min_apps_per_customer.unwrap_or(1).max(1);
    let sort_order = input.sort_order.unwrap_or(0);
    let is_active = input.is_active.unwrap_or(true);

    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO affiliate_rate_bands
             (label, min_paying_customers, min_apps_per_customer, rate, sort_order, is_active)
         VALUES ($1, $2, $3, $4::numeric, $5, $6)
         RETURNING id",
    )
    .bind(&label)
    .bind(min_cust)
    .bind(min_apps)
    .bind(rate)
    .bind(sort_order)
    .bind(is_active)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("create band: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "created": true })),
    ))
}

/// PUT /api/v1/affiliate-rate-bands/:id — edit a band. An absent field keeps its stored value.
pub async fn update_band(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<BandInput>,
) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    if let Some(rate) = input.rate {
        validate_rate(rate)?;
    }
    // A blank label is treated as absent so a partial edit can never store "".
    let label = clean_label(&input.label);
    let min_cust = input.min_paying_customers.map(|v| v.max(0));
    let min_apps = input.min_apps_per_customer.map(|v| v.max(1));

    let res = sqlx::query(
        "UPDATE affiliate_rate_bands
            SET label = COALESCE($2::text, label),
                min_paying_customers = COALESCE($3::int, min_paying_customers),
                min_apps_per_customer = COALESCE($4::int, min_apps_per_customer),
                rate = COALESCE($5::numeric, rate),
                sort_order = COALESCE($6::int, sort_order),
                is_active = COALESCE($7::bool, is_active),
                updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(label)
    .bind(min_cust)
    .bind(min_apps)
    .bind(input.rate)
    .bind(input.sort_order)
    .bind(input.is_active)
    .execute(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("update band: {e}")))?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("band not found".into()));
    }
    Ok(Json(json!({ "id": id, "updated": true })))
}

/// DELETE /api/v1/affiliate-rate-bands/:id — remove a band. Affiliates keep the rate it set.
pub async fn delete_band(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    let res = sqlx::query("DELETE FROM affiliate_rate_bands WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("delete band: {e}")))?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("band not found".into()));
    }
    Ok(Json(json!({ "id": id, "deleted": true })))
}
