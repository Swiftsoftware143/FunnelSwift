//! A referred customer's plan movement — DATED, and what it does to the affiliate's money.
//!
//! This exists because nothing did. `set_active_plan` is the only writer of a tenant's plan in this
//! app, and it touched nothing in the affiliate system: a FunnelSwift customer who upgraded in app
//! credited nobody, no dashboard changed, and no date was recorded. Meanwhile the affiliate guide
//! served from `affiliate_portal_handler` already promised *"a lead can upgrade, downgrade, and
//! upgrade again months later, and you're credited each time."*
//!
//! One entry point (`record`) so there is one definition of what counts as an upgrade and one place
//! that touches the money. Callers: `set_active_plan` (in-app and admin assigns) and the internal
//! upgrade webhook (a sibling app reporting a movement).
//!
//! The timeline table is append-only; `affiliate_commissions` holds only the CURRENT state, dated.
//! A later re-upgrade therefore can never erase the downgrade before it.

use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Movement {
    pub lead_id: Option<Uuid>,
    pub affiliate_id: Option<String>,
    /// 'upgrade' | 'downgrade' | 'start' | 'plan_change'
    pub movement: String,
    pub from_plan: Option<String>,
    pub to_plan: String,
    pub from_price: Option<f64>,
    pub to_price: f64,
    /// The customer is paying now, so the affiliate earns on this movement.
    pub pays: bool,
}

fn classify(from_price: Option<f64>, to_price: f64) -> &'static str {
    match from_price {
        None => "start",
        Some(f) if to_price > f + 0.001 => "upgrade",
        Some(f) if to_price < f - 0.001 => "downgrade",
        _ => "plan_change",
    }
}

/// Everything a movement needs to be recorded. A struct rather than eight positional arguments, and
/// named fields because several of them are optional for different callers.
#[derive(Debug, Clone)]
pub struct Report<'a> {
    /// The tenant whose plan moved, when it is one of ours. `None` for a movement a sibling app
    /// reported — that workspace is not in this database.
    pub tenant_id: Option<Uuid>,
    /// Used to find the lead when `tenant_id` is unknown.
    pub lead_email: Option<&'a str>,
    pub to_plan: &'a str,
    pub to_price: f64,
    /// The plan being left, read by the caller BEFORE it swaps, so this can never see its own result
    /// and report a no-op.
    pub from: Option<(String, f64)>,
    /// Set only when an external sender reported this movement, so a retry cannot produce a second
    /// timeline entry.
    pub event_key: Option<&'a str>,
    /// Set only when the sender tells us the direction ("upgrade"/"downgrade"). We cannot work it out
    /// from price alone for a sibling app, because we never see the plan it left.
    pub force_movement: Option<&'a str>,
}

/// Record a plan movement and settle the affiliate's commission.
pub async fn record(pool: &PgPool, r: Report<'_>) -> Result<Option<Movement>, String> {
    let Report {
        tenant_id,
        lead_email,
        to_plan: to_plan_slug,
        to_price,
        from,
        event_key,
        force_movement,
    } = r;
    // A movement with no plan change at all is not a movement. Re-saving the same plan must not add
    // a line to the affiliate's history.
    if let Some((ref fslug, fprice)) = from {
        if fslug == to_plan_slug && (fprice - to_price).abs() < 0.001 {
            return Ok(None);
        }
    }

    if let Some(k) = event_key {
        let seen: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM affiliate_plan_movements WHERE event_key = $1)",
        )
        .bind(k)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
        if seen {
            return Ok(None);
        }
    }

    let from_price = from.as_ref().map(|(_, p)| *p);
    let movement = force_movement
        .map(|m| m.to_string())
        .unwrap_or_else(|| classify(from_price, to_price).to_string());
    let from_plan = from.as_ref().map(|(s, _)| s.clone());
    // Money only moves when the customer is on a plan they pay for. A movement onto a free plan is a
    // real, dated event that earns nothing — which is the whole reason a downgrade has to be seen.
    let pays = to_price > 0.0 && movement != "downgrade";

    // ── who is credited: the lead this tenant came from, and the affiliate on that lead ──────────
    // The link is the email the account was provisioned with, matched against the lead that was
    // captured. Same permanent anchor the credit uses (`leads.created_by`), so this can never
    // disagree with who gets paid.
    let lead: Option<(Uuid, Option<Uuid>)> = if let Some(tid) = tenant_id {
        sqlx::query_as(
            "SELECT l.id, l.created_by FROM leads l
              WHERE l.email IN (SELECT email FROM users WHERE tenant_id = $1)
              ORDER BY l.created_at DESC LIMIT 1",
        )
        .bind(tid)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    } else if let Some(e) = lead_email {
        sqlx::query_as(
            "SELECT l.id, l.created_by FROM leads l WHERE l.email = $1
              ORDER BY l.created_at DESC LIMIT 1",
        )
        .bind(e)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    } else {
        None
    };

    let lead_id = lead.as_ref().map(|(id, _)| *id);
    let mut affiliate_id: Option<String> = None;
    if let Some((_, Some(user_id))) = lead {
        affiliate_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM affiliates WHERE user_id = $1 AND is_active = true LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    sqlx::query(
        "INSERT INTO affiliate_plan_movements
           (lead_id, tenant_id, affiliate_id, movement, from_plan, to_plan, from_price, to_price, event_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(lead_id)
    .bind(tenant_id)
    .bind(&affiliate_id)
    .bind(&movement)
    .bind(&from_plan)
    .bind(to_plan_slug)
    .bind(from_price)
    .bind(to_price)
    .bind(event_key)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // ── settle the money ────────────────────────────────────────────────────────────────────────
    if let Some(lid) = lead_id {
        if pays {
            // Earned — and re-derived from the price being paid now, so an estimate taken at capture
            // time does not become the number the affiliate is actually owed.
            let r = sqlx::query(
                "UPDATE affiliate_commissions
                    SET status = 'earned',
                        earned_at = NOW(),
                        reversed_at = NULL,
                        amount = CASE WHEN metadata->>'rate' IS NOT NULL
                                      THEN round($2::numeric * (metadata->>'rate')::numeric / 100, 2)
                                      ELSE amount END
                  WHERE lead_id = $1 AND status IN ('pending', 'reversed', 'earned')",
            )
            .bind(lid)
            .bind(to_price)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            let settled = r.rows_affected();
            info!(tenant = ?tenant_id, %lid, movement = %movement, rows = settled,
                  "affiliate commission EARNED on this movement");
        } else if from_price.map(|p| p > 0.0).unwrap_or(false) || movement == "downgrade" {
            // Downgraded off a paying plan — the money stops here, and the date says when.
            let r = sqlx::query(
                "UPDATE affiliate_commissions
                    SET status = 'reversed', reversed_at = NOW()
                  WHERE lead_id = $1 AND status IN ('pending', 'earned')",
            )
            .bind(lid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            let settled = r.rows_affected();
            info!(tenant = ?tenant_id, %lid, rows = settled,
                  "affiliate commission REVERSED — the customer left a paying plan");
        }
    } else if pays {
        warn!(tenant = ?tenant_id, %to_plan_slug,
              "a tenant moved onto a paying plan but no lead traces back to it — nobody is credited");
    }

    Ok(Some(Movement {
        lead_id,
        affiliate_id,
        movement,
        from_plan,
        to_plan: to_plan_slug.to_string(),
        from_price,
        to_price,
        pays,
    }))
}

/// The plan a tenant is on right now: `(slug, price)`. Read BEFORE a swap so the caller can pass it
/// in as `from`.
pub async fn current_plan(pool: &PgPool, tenant_id: Uuid) -> Option<(String, f64)> {
    sqlx::query_as::<_, (String, f64)>(
        "SELECT p.slug, COALESCE(p.price, 0)::float8 FROM tenant_plan_subscriptions tps
           JOIN plans p ON p.id = tps.plan_id
          WHERE tps.tenant_id = $1 AND tps.status = 'active'
          ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// The dated history for one affiliate, newest first — what the affiliate's own dashboard and the
/// admin's screen both read.
pub async fn timeline_for_affiliate(
    pool: &PgPool,
    affiliate_id: &str,
    limit: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let rows: Vec<(Uuid, Option<Uuid>, String, Option<String>, String, Option<f64>, f64, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT m.id, m.lead_id, m.movement, m.from_plan, m.to_plan, m.from_price, m.to_price, m.occurred_at
               FROM affiliate_plan_movements m
              WHERE m.affiliate_id = $1
              ORDER BY m.occurred_at DESC LIMIT $2",
        )
        .bind(affiliate_id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(
            |(id, lead_id, movement, from_plan, to_plan, from_price, to_price, at)| {
                serde_json::json!({
                    "id": id.to_string(),
                    "lead_id": lead_id.map(|l| l.to_string()),
                    "movement": movement,
                    "from_plan": from_plan,
                    "to_plan": to_plan,
                    "from_price": from_price,
                    "to_price": to_price,
                    "occurred_at": at,
                    "date": at.format("%Y-%m-%d").to_string(),
                })
            },
        )
        .collect())
}
