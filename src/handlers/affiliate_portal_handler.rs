use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn affiliate_signup(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = payload["name"].as_str().unwrap_or("Affiliate");
    let email = payload["email"].as_str().ok_or_else(|| AppError::Validation("Email required".into()))?;
    let affiliate_id = Uuid::new_v4().to_string().replace("-", "")[..8].to_uppercase();
    let tenant_id = Uuid::parse_str(
        payload["tenant_id"].as_str().unwrap_or("00000000-0000-0000-0000-000000000001")
    ).map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("INSERT INTO affiliates (id, tenant_id, name, email, commission_rate, is_active) VALUES ($1, $2, $3, $4, 10.0, true)")
        .bind(&affiliate_id).bind(tenant_id).bind(name).bind(email)
        .execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": affiliate_id, "message": "Affiliate account created"}))))
}

pub async fn affiliate_login(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let email = payload["email"].as_str().ok_or_else(|| AppError::Validation("Email required".into()))?;
    let row: (String, String, String, bool) = sqlx::query_as(
        "SELECT id, name, email, is_active FROM affiliates WHERE email = $1"
    ).bind(email).fetch_optional(&state.pool).await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;
    Ok((StatusCode::OK, Json(json!({"id": row.0, "name": row.1, "email": row.2, "is_active": row.3}))))
}

pub async fn affiliate_portal_dashboard(
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let affiliate_id = payload["affiliate_id"].as_str().unwrap_or("");
    let row: (i64, Option<f64>) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(amount), 0) FROM affiliate_commissions WHERE affiliate_id = $1"
    ).bind(affiliate_id).fetch_one(&state.pool).await?;
    Ok(Json(json!({"total_leads": row.0, "total_earnings": row.1.unwrap_or(0.0)})))
}
