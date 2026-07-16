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
    let products: Vec<serde_json::Value> = sqlx::query_as::<_, (serde_json::Value,)>(r#"
            SELECT row_to_json(t.*)::jsonb FROM (
                SELECT ap.id, ap.tenant_id, ap.name, ap.description, ap.price::text, ap.default_commission_rate::text, ap.is_active, ap.is_third_party, ap.url, ap.category_id, ap.created_at, ap.updated_at,
                    pc.name as category_name, pc.slug as category_slug
                FROM affiliate_products ap
                LEFT JOIN product_categories pc ON pc.id = ap.category_id
                WHERE ap.is_active = true
                ORDER BY pc.sort_order, ap.name
            ) t
        "#)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(r,)| r)
    .collect();
    Ok(Json(json!({"data": products})))
}

pub async fn list_all_affiliate_products_admin(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let products: Vec<serde_json::Value> = sqlx::query_as::<_, (serde_json::Value,)>(r#"
            SELECT row_to_json(t.*)::jsonb FROM (
                SELECT ap.id, ap.tenant_id, ap.name, ap.description, ap.price::text, ap.default_commission_rate::text, ap.is_active, ap.is_third_party, ap.url, ap.category_id, ap.created_at, ap.updated_at,
                    pc.name as category_name, pc.slug as category_slug
                FROM affiliate_products ap
                LEFT JOIN product_categories pc ON pc.id = ap.category_id
                ORDER BY pc.sort_order, ap.name
            ) t
        "#)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(r,)| r)
    .collect();
    Ok(Json(json!({"data": products})))
}

pub async fn create_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAffiliateProductRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let product = sqlx::query_as::<_, (serde_json::Value,)>(r#"
        INSERT INTO affiliate_products (tenant_id, name, description, price, default_commission_rate, category_id, url, is_third_party)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING row_to_json(affiliate_products.*)::jsonb
    "#)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price)
    .bind(req.default_commission_rate)
    .bind(req.category_id)
    .bind(&req.url)
    .bind(req.is_third_party.unwrap_or(false))
    .fetch_one(&state.pool)
    .await
    .map_err(|_| AppError::BadRequest("Failed to create product".into()))?;
    Ok(Json(json!({"data": product})))
}

pub async fn update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAffiliateProductRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let product = sqlx::query_as::<_, (serde_json::Value,)>(r#"
        UPDATE affiliate_products SET
            name = COALESCE($3, name),
            description = COALESCE($4, description),
            price = COALESCE($5, price),
            default_commission_rate = COALESCE($6, default_commission_rate),
            is_active = COALESCE($7, is_active),
            category_id = COALESCE($8, category_id),
            url = COALESCE($9, url),
            is_third_party = COALESCE($10, is_third_party),
            updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING row_to_json(affiliate_products.*)::jsonb
    "#)
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price)
    .bind(req.default_commission_rate)
    .bind(req.is_active)
    .bind(req.category_id)
    .bind(&req.url)
    .bind(req.is_third_party)
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
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    sqlx::query("DELETE FROM affiliate_products WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Product deleted"})))
}
