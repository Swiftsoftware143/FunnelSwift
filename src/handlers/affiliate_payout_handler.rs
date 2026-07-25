use axum::{extract::{Path, State}, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::AppState;
use chrono::NaiveDateTime;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateTier {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub min_conversions: i32,
    pub min_sales: f64,
    pub commission_rate: f64,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
}

pub async fn list_tiers(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let tiers = sqlx::query_as::<_, AffiliateTier>(
        "SELECT id, tenant_id, name, min_conversions, min_sales, commission_rate, is_active, created_at FROM affiliate_tiers WHERE tenant_id = $1 ORDER BY min_sales ASC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": tiers})))
}

pub async fn create_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let name = req["name"].as_str().ok_or(AppError::BadRequest("name required".into()))?;
    let rate = req["commission_rate"].as_f64().ok_or(AppError::BadRequest("commission_rate required".into()))?;
    let min_conv = req["min_conversions"].as_i64().unwrap_or(0) as i32;
    let min_sales = req["min_sales"].as_f64().unwrap_or(0.0);
    
    let tier = sqlx::query_as::<_, AffiliateTier>(
        "INSERT INTO affiliate_tiers (tenant_id, name, min_conversions, min_sales, commission_rate)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, tenant_id, name, min_conversions, min_sales, commission_rate, is_active, created_at"
    )
    .bind(tenant_id)
    .bind(name)
    .bind(min_conv)
    .bind(min_sales)
    .bind(rate)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"data": tier})))
}

pub async fn update_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let name = req["name"].as_str();
    let rate = req["commission_rate"].as_f64();
    let min_conv = req["min_conversions"].as_i64();
    let min_sales = req["min_sales"].as_f64();
    let is_active = req["is_active"].as_bool();
    
    if let Some(n) = name { sqlx::query("UPDATE affiliate_tiers SET name=$1 WHERE id=$2 AND tenant_id=$3").bind(n).bind(id).bind(tenant_id).execute(&state.pool).await?; }
    if let Some(r) = rate { sqlx::query("UPDATE affiliate_tiers SET commission_rate=$1 WHERE id=$2 AND tenant_id=$3").bind(r).bind(id).bind(tenant_id).execute(&state.pool).await?; }
    if let Some(m) = min_conv { sqlx::query("UPDATE affiliate_tiers SET min_conversions=$1 WHERE id=$2 AND tenant_id=$3").bind(m as i32).bind(id).bind(tenant_id).execute(&state.pool).await?; }
    if let Some(s) = min_sales { sqlx::query("UPDATE affiliate_tiers SET min_sales=$1 WHERE id=$2 AND tenant_id=$3").bind(s).bind(id).bind(tenant_id).execute(&state.pool).await?; }
    if let Some(a) = is_active { sqlx::query("UPDATE affiliate_tiers SET is_active=$1 WHERE id=$2 AND tenant_id=$3").bind(a).bind(id).bind(tenant_id).execute(&state.pool).await?; }
    
    let tier = sqlx::query_as::<_, AffiliateTier>(
        "SELECT id, tenant_id, name, min_conversions, min_sales, commission_rate, is_active, created_at FROM affiliate_tiers WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"data": tier})))
}

pub async fn delete_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM affiliate_tiers WHERE id=$1 AND tenant_id=$2")
        .bind(id).bind(tenant_id).execute(&state.pool).await?;
    Ok(Json(json!({"success": true})))
}

// Assign tier to affiliate based on performance
pub async fn calculate_affiliate_tier(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(affiliate_id): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
    // Count conversions and total sales for this affiliate
    let conversions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_conversions WHERE affiliate_id = $1 AND tenant_id = $2 AND status = 'paid'"
    )
    .bind(&affiliate_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    let total_sales: Option<f64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(commission_amount), 0) FROM affiliate_conversions WHERE affiliate_id = $1 AND tenant_id = $2 AND status = 'paid'"
    )
    .bind(&affiliate_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    // Find best matching tier
    let best_tier: Option<(Uuid, f64)> = sqlx::query_as(
        "SELECT id, commission_rate FROM affiliate_tiers WHERE tenant_id = $1 AND is_active = true AND min_conversions <= $2 AND min_sales <= $3 ORDER BY min_sales DESC, min_conversions DESC LIMIT 1"
    )
    .bind(tenant_id)
    .bind(conversions as i32)
    .bind(total_sales.unwrap_or(0.0))
    .fetch_optional(&state.pool)
    .await?;
    
    match best_tier {
        Some((tier_id, rate)) => {
            sqlx::query("UPDATE affiliates SET tier_id = $1, commission_rate = $2 WHERE id = $3 AND tenant_id = $4")
                .bind(tier_id).bind(rate).bind(&affiliate_id).bind(tenant_id)
                .execute(&state.pool).await?;
            Ok(Json(json!({"tier_id": tier_id, "commission_rate": rate, "conversions": conversions, "total_sales": total_sales})))
        }
        None => Ok(Json(json!({"tier_id": null, "commission_rate": null, "conversions": conversions, "total_sales": total_sales}))),
    }
}

// ===== PAYOUTS =====

pub async fn list_payouts(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let payouts = sqlx::query_as::<_, (Uuid, String, String, f64, Option<String>, String, Option<NaiveDateTime>, NaiveDateTime)>(
        "SELECT p.id, p.affiliate_id, COALESCE(a.name, a.email, p.affiliate_id), p.amount, p.method, p.status, p.paid_at, p.created_at
         FROM affiliate_payouts p LEFT JOIN affiliates a ON p.affiliate_id = a.id
         WHERE p.tenant_id = $1 ORDER BY p.created_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": payouts})))
}

#[derive(Deserialize)]
pub struct CreatePayoutRequest {
    pub affiliate_id: String,
    pub amount: f64,
    pub method: Option<String>,
    pub conversion_ids: Option<Vec<Uuid>>,
}

pub async fn create_payout(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePayoutRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
    let payout = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO affiliate_payouts (tenant_id, affiliate_id, amount, method, status)
         VALUES ($1, $2, $3, $4, 'pending')
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&req.affiliate_id)
    .bind(req.amount)
    .bind(&req.method)
    .fetch_one(&state.pool)
    .await?;
    
    // Mark conversions as paid
    if let Some(conv_ids) = &req.conversion_ids {
        for conv_id in conv_ids {
            sqlx::query("UPDATE affiliate_conversions SET status = 'paid', paid_at = NOW(), payout_id = $1 WHERE id = $2")
                .bind(payout.0)
                .bind(conv_id)
                .execute(&state.pool)
                .await?;
        }
    }
    
    Ok(Json(json!({"success": true, "payout_id": payout.0})))
}

pub async fn mark_payout_paid(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
    sqlx::query("UPDATE affiliate_payouts SET status = 'paid', paid_at = NOW() WHERE id = $1 AND tenant_id = $2")
        .bind(id).bind(tenant_id).execute(&state.pool).await?;
    
    // Also mark all associated conversions as paid
    sqlx::query("UPDATE affiliate_conversions SET status = 'paid', paid_at = NOW() WHERE payout_id = $1")
        .bind(id).execute(&state.pool).await?;
    
    Ok(Json(json!({"success": true})))
}

pub async fn get_affiliate_pending_conversions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(affiliate_id): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
    let conversions = sqlx::query_as::<_, (Uuid, String, f64, String, NaiveDateTime)>(
        "SELECT id, COALESCE(source_app, 'unknown'), commission_amount, status, converted_at
         FROM affiliate_conversions WHERE affiliate_id = $1 AND tenant_id = $2 AND status = 'pending'
         ORDER BY converted_at DESC"
    )
    .bind(&affiliate_id)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    
    let total_pending: f64 = conversions.iter().map(|(_, _, a, _, _)| a).sum();
    
    Ok(Json(json!({"data": conversions, "total_pending": total_pending})))
}
