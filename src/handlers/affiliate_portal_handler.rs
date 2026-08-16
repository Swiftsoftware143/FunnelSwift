use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, response::Html, Json};
use serde_json::{json, Value};
use uuid::Uuid;

// Affiliates are regular users — there is NO separate affiliate login.
// "Become an affiliate" is an opt-in flag on the authenticated user's account,
// auto-approved by the system. Payout rate is derived from the user's plan tier.

pub async fn affiliate_signup(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Idempotent: a user is an affiliate at most once per tenant.
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM affiliates WHERE email = $1 AND tenant_id = $2")
            .bind(&auth.email)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    if let Some(id) = existing {
        return Ok((
            StatusCode::OK,
            Json(json!({"id": id, "message": "Affiliate account already exists"})),
        ));
    }

    // Effective payout = the user's active plan's commission_rate (admin-adjustable per plan).
    let plan_rate: Option<f64> = sqlx::query_scalar(
        "SELECT p.commission_rate::float8 FROM plans p
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
         WHERE tps.tenant_id = $1 AND tps.status = 'active'
         ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let commission_rate = plan_rate.unwrap_or(20.0);

    let affiliate_id = Uuid::new_v4().to_string().replace('-', "")[..8].to_uppercase();
    // Link the affiliate record to the user account (commission is tracked on the user account).
    let user_id = Uuid::parse_str(&auth.user_id).ok();
    sqlx::query(
        "INSERT INTO affiliates (id, tenant_id, name, email, commission_rate, is_active, user_id) VALUES ($1, $2, $3, $4, $5, true, $6)",
    )
    .bind(&affiliate_id)
    .bind(tenant_id)
    .bind(&auth.email)
    .bind(&auth.email)
    .bind(commission_rate)
    .bind(user_id)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": affiliate_id, "message": "Affiliate account created"})),
    ))
}

pub async fn affiliate_portal_dashboard(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Resolve the affiliate identity from the authenticated user, never the request body.
    let affiliate_id: String =
        sqlx::query_scalar("SELECT id FROM affiliates WHERE email = $1 AND tenant_id = $2")
            .bind(&auth.email)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Affiliate account not found".into()))?;

    let row: (i64, Option<f64>) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(amount), 0)::float8 FROM affiliate_commissions WHERE affiliate_id = $1",
    )
    .bind(&affiliate_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(json!({
        "affiliate_id": affiliate_id,
        "total_leads": row.0,
        "total_earnings": row.1.unwrap_or(0.0),
    })))
}

/// Non-technical affiliate guide, served only behind the affiliate request gate:
/// the caller must be an authenticated user who has an `affiliates` record.
pub async fn affiliate_guide(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Html<String>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // The request gate: only users who have requested affiliate status.
    let affiliate: Option<String> =
        sqlx::query_scalar("SELECT id FROM affiliates WHERE email = $1 AND tenant_id = $2")
            .bind(&auth.email)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    if affiliate.is_none() {
        return Err(AppError::Forbidden(
            "Request affiliate status to view this guide.".into(),
        ));
    }

    Ok(Html(AFFILIATE_GUIDE_HTML.to_string()))
}

const AFFILIATE_GUIDE_HTML: &str = r#"<style>
#aff-guide{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;color:#1e293b;line-height:1.6}
#aff-guide h2{font-size:20px;font-weight:700;margin-bottom:4px}
#aff-guide .sub{color:#64748b;font-size:13px;margin-bottom:20px}
#aff-guide .gcard{background:#fff;border-radius:10px;padding:16px 18px;box-shadow:0 1px 3px rgba(0,0,0,.06);margin-bottom:14px}
#aff-guide .gcard h3{font-size:14px;font-weight:600;margin-bottom:8px;color:#2563eb}
#aff-guide ul,#aff-guide ol{padding-left:20px;margin:0}
#aff-guide li{margin:4px 0}
#aff-guide table{width:100%;border-collapse:collapse;font-size:13px}
#aff-guide th,#aff-guide td{text-align:left;padding:7px 10px;border-bottom:1px solid #e2e8f0}
#aff-guide th{color:#64748b;font-size:11px;text-transform:uppercase;letter-spacing:.05em}
#aff-guide .note{font-size:12px;color:#94a3b8;margin-top:16px;text-align:center}
</style>
<div id="aff-guide">
<h2>Affiliate Guide</h2>
<div class="sub">Plain-language guide for FunnelSwift affiliates &mdash; no technical setup required.</div>

<div class="gcard">
<h3>What is the Affiliate Program?</h3>
<p>FunnelSwift pays you a commission for referring people to the Swift products &mdash; FunnelSwift itself, plus CoreSwift, WorkflowSwift, IncentiveSwift, ADASwift, and the rest.</p>
<p>It works through the leads you already run through FunnelSwift:</p>
<ol>
<li>You bring leads through your FunnelSwift account.</li>
<li>When a lead gets connected to a Swift product, they receive a <strong>free account</strong> in that product.</li>
<li>If that lead later <strong>upgrades to a paid plan</strong>, you earn a commission.</li>
<li>You keep earning on that lead <strong>every time they upgrade &mdash; forever.</strong> There is no cookie expiry and no time limit.</li>
</ol>
</div>

<div class="gcard">
<h3>Becoming an Affiliate</h3>
<ol>
<li>Log in to your normal FunnelSwift account (no separate login).</li>
<li>Go to the <strong>Affiliate</strong> section of your portal.</li>
<li>Click <strong>Become an Affiliate</strong> and accept the terms.</li>
<li>You're <strong>approved automatically</strong> &mdash; no manual review.</li>
</ol>
</div>

<div class="gcard">
<h3>How You Earn</h3>
<p>Your payout rate is tied to the plan your own account is on:</p>
<table>
<tr><th>Plan</th><th>Payout %</th></tr>
<tr><td>Free (Capture / Kinetic)</td><td>20%</td></tr>
<tr><td>Starter / Pro</td><td>30%</td></tr>
<tr><td>Suite</td><td>40%</td></tr>
<tr><td>Agency</td><td>50%</td></tr>
</table>
<p style="margin-top:8px;font-size:13px">Upgrade your own plan and your payout rate increases automatically.</p>
</div>

<div class="gcard">
<h3>What Counts as a Commission</h3>
<p>Every time a lead that came through your account upgrades to a paid plan in any Swift product, you're credited automatically. You don't fill anything out &mdash; it happens in the background. A lead can upgrade, downgrade, and upgrade again months later, and you're credited each time.</p>
</div>

<div class="gcard">
<h3>Tracking Your Earnings</h3>
<p>Open the <strong>Affiliate</strong> section of your portal any time to see your leads and total earnings.</p>
</div>

<div class="note">FunnelSwift &mdash; The SwiftSoftware Affiliate Hub</div>
</div>"#;
