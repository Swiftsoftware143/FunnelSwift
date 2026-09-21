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
        "SELECT id, name, commission_rate, min_sales, min_revenue, description, created_at FROM affiliate_tiers ORDER BY min_sales ASC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| tracing::error!("list_tiers query failed: {:?}", e))
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
    // `affiliate_payouts` (migration 021) is id / affiliate_user_id / amount / method / status /
    // paid_at. The old SELECT asked for p.affiliate_id, p.period and p.created_at -- none of which
    // exist -- so the query errored on EVERY call and unwrap_or_default() reported the failure as
    // `200 []` (kanban t_4385aa2c). amount is NUMERIC while f64 decodes FLOAT8 only, hence the
    // cast; the join keys on affiliates.user_id (affiliates.id is varchar, this column is uuid).
    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        Option<f64>,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT p.id, p.affiliate_user_id, p.amount::float8 AS amount, p.status, p.method, p.paid_at, a.name AS affiliate_name \
         FROM affiliate_payouts p LEFT JOIN affiliates a ON a.user_id = p.affiliate_user_id \
         ORDER BY p.paid_at DESC NULLS LAST, p.id",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("list_payouts query failed: {:?}", e);
        e
    })?;
    let payouts: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.0.to_string(),
                "affiliate_user_id": r.1.map(|u| u.to_string()),
                // legacy key kept for the shipped SPA (`p.affiliate_name || p.affiliate_id`)
                "affiliate_id": r.1.map(|u| u.to_string()),
                "amount": r.2,
                "status": r.3,
                "method": r.4,
                "paid_at": r.5,
                // no period column exists in affiliate_payouts; keep the key the SPA reads
                "period": serde_json::Value::Null,
                "affiliate_name": r.6,
            })
        })
        .collect();
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
    // same column drift as list_payouts: this INSERT named affiliate_id/period, which do not
    // exist on affiliate_payouts, so POST /affiliate-payouts always 500'd (kanban t_4385aa2c)
    let affiliate_user_id = req["affiliate_user_id"]
        .as_str()
        .or_else(|| req["affiliate_id"].as_str())
        .ok_or_else(|| AppError::BadRequest("affiliate_user_id is required".into()))?;
    let affiliate_user_id: Uuid = affiliate_user_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid affiliate_user_id".into()))?;
    let amount = req["amount"].as_f64().unwrap_or(0.0);
    let status = req["status"].as_str().unwrap_or("pending");
    let method = req["method"].as_str();

    sqlx::query(
        "INSERT INTO affiliate_payouts (id, affiliate_user_id, amount, method, status) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(affiliate_user_id)
    .bind(amount)
    .bind(method)
    .bind(status)
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
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    // Return the affiliate's actual payout rate (plan-derived) plus accrued earnings.
    let row: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT commission_rate::float8, (SELECT SUM(amount)::float8 FROM affiliate_commissions WHERE affiliate_id = $1) FROM affiliates WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        Some((rate, earned)) => Ok(Json(json!({
            "affiliate_id": id,
            "commission_rate": rate.unwrap_or(0.0),
            "total_earned": earned.unwrap_or(0.0),
        }))),
        None => Err(AppError::NotFound("Affiliate not found".into())),
    }
}
