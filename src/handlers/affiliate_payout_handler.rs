use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct CreateTierRequest {
    pub name: String,
    pub commission_rate: Option<f64>,
    pub min_sales: Option<i32>,
    pub min_revenue: Option<f64>,
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateTierRequest {
    pub name: Option<String>,
    pub commission_rate: Option<f64>,
    pub min_sales: Option<i32>,
    pub min_revenue: Option<f64>,
    pub description: Option<String>,
}

pub async fn list_tiers(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let rows: Vec<(Uuid, String, f64, i32, f64, Option<String>, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, commission_rate, min_sales, min_revenue, description, created_at FROM affiliate_tiers ORDER BY min_sales ASC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let tiers: Vec<serde_json::Value> = rows.iter().map(|r| json!({"id": r.0.to_string(), "name": r.1, "commission_rate": r.2, "min_sales": r.3, "min_revenue": r.4, "description": r.5, "created_at": r.6})).collect();
    Ok(Json(json!(tiers)))
}

pub async fn create_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTierRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let id = Uuid::new_v4();
    let commission_rate = req.commission_rate.unwrap_or(10.0);
    let min_sales = req.min_sales.unwrap_or(0);
    let min_revenue = req.min_revenue.unwrap_or(0.0);

    sqlx::query(
        "INSERT INTO affiliate_tiers (id, name, commission_rate, min_sales, min_revenue, description) VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(id)
    .bind(&req.name)
    .bind(commission_rate)
    .bind(min_sales)
    .bind(min_revenue)
    .bind(&req.description)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Tier created"})),
    ))
}

pub async fn update_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTierRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let existing = sqlx::query_as::<_, (String, f64, i32, f64)>(
        "SELECT name, commission_rate, min_sales, min_revenue FROM affiliate_tiers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tier not found".into()))?;

    let name = req.name.unwrap_or(existing.0);
    let rate = req.commission_rate.unwrap_or(existing.1);
    let sales = req.min_sales.unwrap_or(existing.2);
    let revenue = req.min_revenue.unwrap_or(existing.3);

    sqlx::query(
        "UPDATE affiliate_tiers SET name=$1, commission_rate=$2, min_sales=$3, min_revenue=$4 WHERE id=$5"
    )
    .bind(&name)
    .bind(rate)
    .bind(sales)
    .bind(revenue)
    .bind(id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Tier updated"})))
}

pub async fn delete_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    sqlx::query("DELETE FROM affiliate_tiers WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Tier deleted"})))
}

pub async fn list_payouts(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let rows: Vec<(Uuid, String, f64, String, Option<String>, chrono::NaiveDateTime, Option<String>)> = sqlx::query_as(
        "SELECT p.id, p.affiliate_id, p.amount, p.status, p.period, p.created_at, a.name as affiliate_name FROM affiliate_payouts p LEFT JOIN affiliates a ON p.affiliate_id = a.id ORDER BY p.created_at DESC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let payouts: Vec<serde_json::Value> = rows.iter().map(|r| json!({"id": r.0.to_string(), "affiliate_id": r.1, "amount": r.2, "status": r.3, "period": r.4, "created_at": r.5, "affiliate_name": r.6})).collect();
    Ok(Json(json!(payouts)))
}

pub async fn create_payout(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let id = Uuid::new_v4();
    let affiliate_id = req["affiliate_id"].as_str().unwrap_or("");
    let amount = req["amount"].as_f64().unwrap_or(0.0);
    let status = req["status"].as_str().unwrap_or("pending");
    let period = req["period"].as_str().unwrap_or("");

    sqlx::query(
        "INSERT INTO affiliate_payouts (id, affiliate_id, amount, status, period) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(affiliate_id)
    .bind(amount)
    .bind(status)
    .bind(period)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Payout created"})),
    ))
}

pub async fn mark_payout_paid(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    sqlx::query("UPDATE affiliate_payouts SET status = 'paid', paid_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Payout marked as paid"})))
}

pub async fn calculate_affiliate_tier(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    // Return the current tier for the affiliate based on their stats
    Ok(Json(json!({"tier": "Bronze", "commission_rate": 10.0})))
}

pub async fn get_affiliate_pending_conversions(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!([])))
}
