use axum::{extract::{Path, State}, Json};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::*;
use crate::AppState;
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ProductCategory {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateCategoryRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

pub async fn list_categories(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let categories = sqlx::query_as::<_, (Value,)>(
        "SELECT row_to_json(t.*)::jsonb FROM (SELECT id, name, slug, description, sort_order, is_active, created_at, updated_at FROM product_categories WHERE is_active = true ORDER BY sort_order, name) t"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(r,)| r)
    .collect::<Vec<_>>();
    Ok(Json(json!({"data": categories})))
}

pub async fn create_category(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let category = sqlx::query_as::<_, (Value,)>(
        "INSERT INTO product_categories (tenant_id, name, slug, description, sort_order) VALUES ($1, $2, $3, $4, $5) RETURNING row_to_json(product_categories.*)::jsonb"
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.sort_order.unwrap_or(0))
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
            AppError::BadRequest("A category with this slug already exists".into())
        } else {
            AppError::BadRequest("Failed to create category".into())
        }
    })?;
    Ok(Json(json!({"data": category})))
}

pub async fn update_category(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let category = sqlx::query_as::<_, (Value,)>(
        "UPDATE product_categories SET
            name = COALESCE($3, name),
            slug = COALESCE($4, slug),
            description = COALESCE($5, description),
            sort_order = COALESCE($6, sort_order),
            is_active = COALESCE($7, is_active),
            updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING row_to_json(product_categories.*)::jsonb"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.sort_order)
    .bind(req.is_active)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| AppError::NotFound("Category not found".into()))?;
    Ok(Json(json!({"data": category})))
}

pub async fn delete_category(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM product_categories WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"success": true})))
}
