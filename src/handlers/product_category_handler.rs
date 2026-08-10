// Product category handler
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCategoryRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, sqlx::FromRow)]
struct CategoryRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub created_at: chrono::NaiveDateTime,
}

pub async fn list_categories(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let categories: Vec<CategoryRow> = sqlx::query_as(
        "SELECT id, tenant_id, name, slug, description, sort_order, is_active, created_at FROM product_categories ORDER BY sort_order ASC"
    )
    .fetch_all(&state.pool)
    .await?;

    let result: Vec<Value> = categories
        .iter()
        .map(|c| {
            json!({
                "id": c.id.to_string(),
                "name": c.name,
                "slug": c.slug,
                "description": c.description,
                "sort_order": c.sort_order,
                "is_active": c.is_active,
            })
        })
        .collect();

    Ok(Json(json!(result)))
}

pub async fn create_category(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let id = Uuid::new_v4();
    let slug = req
        .slug
        .unwrap_or_else(|| req.name.to_lowercase().replace(' ', "-"));

    sqlx::query(
        "INSERT INTO product_categories (id, tenant_id, name, slug, description) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&slug)
    .bind(&req.description)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.to_string(), "message": "Category created"})),
    ))
}

pub async fn update_category(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, slug, description FROM product_categories WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    let ex = existing.ok_or_else(|| AppError::NotFound("Category not found".into()))?;

    let name = req.name.unwrap_or(ex.0);
    let slug = req.slug.unwrap_or(ex.1);
    let desc = req.description.or(ex.2);

    sqlx::query("UPDATE product_categories SET name=$1, slug=$2, description=$3 WHERE id=$4 AND tenant_id=$5")
        .bind(&name).bind(&slug).bind(&desc).bind(id).bind(tenant_id)
        .execute(&state.pool).await?;

    Ok(Json(json!({"message": "Category updated"})))
}

pub async fn delete_category(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM product_categories WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Category deleted"})))
}
