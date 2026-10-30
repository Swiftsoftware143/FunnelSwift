//! Reader for `referral_tracking` (kanban t_66cf5241).
//!
//! `POST /api/v1/auth/signup` has always written one row per signup that arrived with an
//! `affiliate_code` (`public_signup_handler.rs`, "Handle affiliate referral code") and nothing in the
//! repository ever read the table: the attribution the public affiliate guide promises
//! ("the system tags them as your referral", "All their paid plan upgrades are attributed to your
//! referral" - `www/guide-affiliate.html`) had no surface at all, so the write was durable but dead.
//! This module is that surface: a per-code referral-signup count plus the signups themselves.
//!
//! Keying: `referral_tracking.referrer_code` is a CODE, not an id, and this schema carries three code
//! namespaces a signup's `?ref=` can legitimately arrive from, so every code is resolved through all
//! of them instead of guessing one:
//!   * `tenants.affiliate_code`         - what `kinetic_handler` renders into the badge `?ref=` URL
//!   * `affiliate_links.tracking_code`  - the per-link code (UNIQUE), joined to its affiliate
//!   * `affiliates.id`                  - shown as the "Code"/"ID" column on the admin Affiliates tab
//!
//! A code that resolves to nothing is still COUNTED and returned (`resolved: false`): a code silently
//! dropped from the report would be the same class of defect this module exists to close.
//!
//! Scope: an admin sees every code; any other caller sees only the referrals their own tenant's code
//! produced (a code they do not own is not their business). The counts live in scalar subqueries, not
//! JOINs, so a duplicate `affiliate_code` across two tenants cannot fan the rows out and inflate
//! `COUNT(*)`.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// How many of the most recent referral signups the report carries. The COUNT is unbounded; this is
/// only the drill-down list.
const RECENT_LIMIT: i64 = 25;

/// Per-code aggregate. `$1` = see-every-code flag (admin), `$2` = the caller's own code.
const BY_CODE_SQL: &str = "\
SELECT rt.referrer_code,
       COUNT(*)::int8 AS signups,
       MAX(rt.created_at) AS last_signup_at,
       (SELECT t.id FROM tenants t WHERE t.affiliate_code = rt.referrer_code ORDER BY t.created_at LIMIT 1) AS referrer_tenant_id,
       (SELECT t.name FROM tenants t WHERE t.affiliate_code = rt.referrer_code ORDER BY t.created_at LIMIT 1) AS referrer_tenant_name,
       (SELECT af.id FROM affiliates af WHERE af.id = COALESCE((SELECT al.affiliate_id FROM affiliate_links al WHERE al.tracking_code = rt.referrer_code LIMIT 1), rt.referrer_code) LIMIT 1) AS affiliate_id,
       (SELECT af.name FROM affiliates af WHERE af.id = COALESCE((SELECT al2.affiliate_id FROM affiliate_links al2 WHERE al2.tracking_code = rt.referrer_code LIMIT 1), rt.referrer_code) LIMIT 1) AS affiliate_name,
       (SELECT af.email FROM affiliates af WHERE af.id = COALESCE((SELECT al3.affiliate_id FROM affiliate_links al3 WHERE al3.tracking_code = rt.referrer_code LIMIT 1), rt.referrer_code) LIMIT 1) AS affiliate_email
  FROM referral_tracking rt
 WHERE ($1::bool OR rt.referrer_code = ANY($2))
 GROUP BY rt.referrer_code
 ORDER BY signups DESC, last_signup_at DESC";

/// The most recent signups, newest first.
const RECENT_SQL: &str = "\
SELECT rt.referrer_code, rt.referred_email, rt.referred_tenant_id, rt.created_at,
       (SELECT t.name FROM tenants t WHERE t.id = rt.referred_tenant_id) AS referred_tenant_name
  FROM referral_tracking rt
 WHERE ($1::bool OR rt.referrer_code = ANY($2))
 ORDER BY rt.created_at DESC
 LIMIT $3";

fn ts(v: chrono::NaiveDateTime) -> String {
    v.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// GET /api/v1/affiliate-referrals - the affiliate-signup attribution written by
/// `POST /api/v1/auth/signup`, read back.
pub async fn list_affiliate_referrals(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // The caller's own shareable code. "No code yet" is a normal state (126 tenants, one code live on
    // this box) and must answer 0 rows rather than an error.
    let own_code: Option<String> =
        sqlx::query_scalar("SELECT affiliate_code FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();

    let scope_all = auth.is_admin;

    // EVERY code this caller owns -- not just the workspace badge code (cron 2026-10-30).
    //
    // A referral row carries whichever code the visitor actually arrived with, and the code an
    // affiliate SHARES is the link code minted on their "Products to Promote" screen
    // (`affiliate_links.tracking_code`), not `tenants.affiliate_code`. Scoping on the tenant code
    // alone therefore answered `total_signups = 0` to the very affiliate who earned the referral:
    // measured live with a self-service affiliate, a minted `TRACK…` code, and a visitor who signed
    // up with `?ref=TRACK…` -- `referral_tracking` held the row and this route hid it, so the
    // affiliate's own screen said nobody had ever signed up through them.
    //
    // The three namespaces are the ones this module already resolves per row: the workspace code,
    // the per-link codes, and the short affiliate id shown on the admin Affiliates tab.
    let own_codes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT code FROM (
             SELECT t.affiliate_code AS code FROM tenants t WHERE t.id = $1
             UNION
             SELECT al.tracking_code FROM affiliate_links al
               JOIN affiliates af ON af.id = al.affiliate_id
              WHERE af.tenant_id = $1 OR af.email = $2 OR af.user_id = $3
             UNION
             SELECT af.id FROM affiliates af
              WHERE af.tenant_id = $1 OR af.email = $2 OR af.user_id = $3
         ) owned
         WHERE code IS NOT NULL AND code <> ''",
    )
    .bind(tenant_id)
    .bind(&auth.email)
    .bind(Uuid::parse_str(&auth.user_id).ok())
    .fetch_all(&state.pool)
    .await?;

    let rows = sqlx::query(BY_CODE_SQL)
        .bind(scope_all)
        .bind(&own_codes)
        .fetch_all(&state.pool)
        .await?;

    let mut by_code = Vec::with_capacity(rows.len());
    let mut total: i64 = 0;
    for r in &rows {
        let code: String = r.try_get("referrer_code")?;
        let signups: i64 = r.try_get("signups")?;
        let last: Option<chrono::NaiveDateTime> = r.try_get("last_signup_at")?;
        let ref_tenant_id: Option<Uuid> = r.try_get("referrer_tenant_id")?;
        let ref_tenant_name: Option<String> = r.try_get("referrer_tenant_name")?;
        let aff_id: Option<String> = r.try_get("affiliate_id")?;
        let aff_name: Option<String> = r.try_get("affiliate_name")?;
        let aff_email: Option<String> = r.try_get("affiliate_email")?;
        total += signups;

        let referrer_tenant = ref_tenant_id.map(|id| {
            json!({"tenant_id": id.to_string(), "name": ref_tenant_name.clone().unwrap_or_default()})
        });
        let affiliate = aff_id.map(|id| {
            json!({
                "affiliate_id": id,
                "name": aff_name.clone().unwrap_or_default(),
                "email": aff_email.clone().unwrap_or_default(),
            })
        });
        by_code.push(json!({
            "referrer_code": code,
            "signups": signups,
            "last_signup_at": last.map(ts),
            "resolved": referrer_tenant.is_some() || affiliate.is_some(),
            "referrer_tenant": referrer_tenant,
            "affiliate": affiliate,
        }));
    }

    let recent_rows = sqlx::query(RECENT_SQL)
        .bind(scope_all)
        .bind(&own_codes)
        .bind(RECENT_LIMIT)
        .fetch_all(&state.pool)
        .await?;

    let mut recent = Vec::with_capacity(recent_rows.len());
    for r in &recent_rows {
        let code: String = r.try_get("referrer_code")?;
        let email: String = r.try_get("referred_email")?;
        let referred_tenant_id: Uuid = r.try_get("referred_tenant_id")?;
        let created_at: chrono::NaiveDateTime = r.try_get("created_at")?;
        let referred_tenant_name: Option<String> = r.try_get("referred_tenant_name")?;
        recent.push(json!({
            "referrer_code": code,
            "referred_email": email,
            "referred_tenant_id": referred_tenant_id.to_string(),
            "referred_tenant_name": referred_tenant_name,
            "created_at": ts(created_at),
        }));
    }

    // The link a tenant shares. Same `?ref=` contract `www/signup.html` reads and `kinetic_handler`
    // renders into the branding badge.
    // The badge link keeps its published contract: the workspace's own code, and nothing when the
    // workspace has none. A link CODE is not substituted here because the console renders this
    // block only when `affiliate_code` is present -- a share_url with no code named beside it would
    // be a surface nothing on screen ever shows.
    let share_url = own_code
        .as_ref()
        .filter(|c| !c.is_empty())
        .map(|c| format!("https://funnelswift.net/signup?ref={c}"));

    Ok(Json(json!({
        "affiliate_code": own_code,
        "share_url": share_url,
        "scope": if scope_all { "all_codes" } else { "own_codes" },
        "total_signups": total,
        "distinct_codes": by_code.len(),
        "by_code": by_code,
        "recent": recent,
    })))
}
