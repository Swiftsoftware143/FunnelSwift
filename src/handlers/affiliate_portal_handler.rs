use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use sqlx::Row;
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

    // Money, split by what it actually is. "Total" alone hid the difference between what has been
    // earned, what is still waiting, and what a customer took back by leaving — David, 2026-10-01.
    let money: (f64, f64, f64) = sqlx::query_as(
        "SELECT
           COALESCE(SUM(amount) FILTER (WHERE status IN ('earned', 'paid')), 0)::float8,
           COALESCE(SUM(amount) FILTER (WHERE status = 'pending'), 0)::float8,
           COALESCE(SUM(amount) FILTER (WHERE status = 'reversed'), 0)::float8
         FROM affiliate_commissions WHERE affiliate_id = $1",
    )
    .bind(&affiliate_id)
    .fetch_one(&state.pool)
    .await?;

    // ── the referred leads: what plan they are on NOW, and WHEN they moved ──────────────────────
    // Before this the dashboard was three numbers and no people: an affiliate could not see who they
    // had referred, whether any of them had upgraded, or when. The plan is read from the account
    // provisioned for that lead's email, and the movement comes from the dated timeline written by
    // `plan_movement` — the same rows the admin's screen reads, so the two can never disagree.
    let user_id: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM affiliates WHERE id = $1")
        .bind(&affiliate_id)
        .fetch_optional(&state.pool)
        .await?
        .flatten();

    let mut referrals: Vec<Value> = Vec::new();
    if let Some(uid) = user_id {
        let rows = sqlx::query(
            "SELECT l.id, l.name, l.email, l.created_at,
                    (SELECT p.slug FROM users u
                       JOIN tenant_plan_subscriptions tps ON tps.tenant_id = u.tenant_id AND tps.status = 'active'
                       JOIN plans p ON p.id = tps.plan_id
                      WHERE u.email = l.email
                      ORDER BY tps.start_date DESC LIMIT 1) AS current_plan,
                    (SELECT COALESCE(p.price, 0)::float8 FROM users u
                       JOIN tenant_plan_subscriptions tps ON tps.tenant_id = u.tenant_id AND tps.status = 'active'
                       JOIN plans p ON p.id = tps.plan_id
                      WHERE u.email = l.email
                      ORDER BY tps.start_date DESC LIMIT 1) AS current_price,
                    (SELECT m.movement FROM affiliate_plan_movements m WHERE m.lead_id = l.id
                      ORDER BY m.occurred_at DESC LIMIT 1) AS last_movement,
                    (SELECT m.occurred_at FROM affiliate_plan_movements m WHERE m.lead_id = l.id
                      ORDER BY m.occurred_at DESC LIMIT 1) AS last_movement_at,
                    (SELECT c.status FROM affiliate_commissions c WHERE c.lead_id = l.id
                      ORDER BY c.created_at DESC LIMIT 1) AS commission_status,
                    (SELECT COALESCE(SUM(c.amount), 0)::float8 FROM affiliate_commissions c
                      WHERE c.lead_id = l.id AND c.status IN ('earned', 'paid')) AS earned
               FROM leads l
              WHERE l.created_by = $1
              ORDER BY l.created_at DESC LIMIT 100",
        )
        .bind(uid)
        .fetch_all(&state.pool)
        .await?;

        referrals = rows
            .iter()
            .map(|r| {
                let last: Option<chrono::DateTime<chrono::Utc>> =
                    r.try_get("last_movement_at").ok().flatten();
                let mv: Option<String> = r.try_get("last_movement").ok().flatten();
                json!({
                    "lead_id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
                    "name": r.try_get::<Option<String>, _>("name").ok().flatten(),
                    "email": r.try_get::<Option<String>, _>("email").ok().flatten(),
                    "joined_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").ok(),
                    "plan": r.try_get::<Option<String>, _>("current_plan").ok().flatten(),
                    "pays": r.try_get::<Option<f64>, _>("current_price").ok().flatten().unwrap_or(0.0) > 0.0,
                    "last_movement": mv,
                    "last_movement_at": last,
                    "last_movement_date": last.map(|d| d.format("%Y-%m-%d").to_string()),
                    "commission_status": r.try_get::<Option<String>, _>("commission_status").ok().flatten(),
                    "earned": format!("{:.2}", r.try_get::<f64, _>("earned").unwrap_or(0.0)),
                })
            })
            .collect();
    }

    // A swallowed error here was a live defect (kanban t_6b43b759): `unwrap_or_default()` turned a
    // DECODE failure into "no plan movements, ever" for every affiliate — measured on the deployed
    // binary 2026-10-02, this arm answered `movements: []` while the row was in the table. The
    // cause was next door in `timeline_for_affiliate`: it read two NUMERIC columns as f64 without
    // the `::float8` cast every other numeric read in this app uses. Both halves are fixed in one
    // change — the cast makes the statement sound, and a failure is now a 500 the caller can see
    // rather than a silently empty history.
    let timeline = crate::plan_movement::timeline_for_affiliate(&state.pool, &affiliate_id, 100)
        .await
        .map_err(AppError::Internal)?;

    Ok(Json(json!({
        "affiliate_id": affiliate_id,
        "total_leads": referrals.len() as i64,
        "total_earnings": row.1.unwrap_or(0.0),
        "earned": format!("{:.2}", money.0),
        "pending": format!("{:.2}", money.1),
        "reversed": format!("{:.2}", money.2),
        "commission_rows": row.0,
        "referrals": referrals,
        "movements": timeline,
    })))
}
