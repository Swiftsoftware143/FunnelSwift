use axum::{extract::{Path, State}, Json};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::models::affiliate_product::*;
use crate::error::AppError;
use crate::error::AppResult;
use crate::AppState;
use serde_json::json;

pub async fn list_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let products = sqlx::query_as::<_, AffiliateProduct>(
        "SELECT id, tenant_id, name, description, price, default_commission_rate, is_active, created_at, updated_at 
         FROM affiliate_products WHERE tenant_id = $1 AND is_active = true ORDER BY name"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": products})))
}

pub async fn create_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAffiliateProductRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let product = sqlx::query_as::<_, AffiliateProduct>(
        "INSERT INTO affiliate_products (tenant_id, name, description, price, default_commission_rate)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, tenant_id, name, description, price, default_commission_rate, is_active, created_at, updated_at"
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price)
    .bind(req.default_commission_rate)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"data": product})))
}

pub async fn update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAffiliateProductRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let product = sqlx::query_as::<_, AffiliateProduct>(
        "UPDATE affiliate_products SET 
            name = COALESCE($3, name),
            description = COALESCE($4, description),
            price = COALESCE($5, price),
            default_commission_rate = COALESCE($6, default_commission_rate),
            is_active = COALESCE($7, is_active),
            updated_at = NOW()
         WHERE id = $1 AND tenant_id = $2
         RETURNING id, tenant_id, name, description, price, default_commission_rate, is_active, created_at, updated_at"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price)
    .bind(req.default_commission_rate)
    .bind(req.is_active)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"data": product})))
}

pub async fn delete_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM affiliate_products WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Product deleted"})))
}
