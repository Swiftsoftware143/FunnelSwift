use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
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

    let affiliate_id = Uuid::new_v4().to_string().replace('-', "")[..8].to_uppercase();
    sqlx::query(
        "INSERT INTO affiliates (id, tenant_id, name, email, commission_rate, is_active) VALUES ($1, $2, $3, $4, 10.0, true)",
    )
    .bind(&affiliate_id)
    .bind(tenant_id)
    .bind(&auth.email)
    .bind(&auth.email)
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
        "SELECT COUNT(*), COALESCE(SUM(amount), 0) FROM affiliate_commissions WHERE affiliate_id = $1",
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
