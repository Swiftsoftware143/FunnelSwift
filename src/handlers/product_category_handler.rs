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
    // NULLABLE-DECODED-AS-NON-OPTION (struct), kanban t_d5da34d0. Every column below is NULLABLE,
    // so ONE row carrying a NULL failed the whole-row decode of this list with "unexpected null;
    // try decoding as an Option". The three columns with no DEFAULT carry real data when NULL
    // (the 8 tenant-less seed rows are the live NULL slug/tenant_id population) and decode as
    // Option; the two DEFAULT'd scalars keep their Rust type through COALESCE in the SELECT;
    // `created_at` stays Option because a `now()` DEFAULT is an insertion-time fact, not a value
    // for an unset column (the same call as t_d79eb91c's provider_keys.created_at).
    pub tenant_id: Option<Uuid>,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    // product_categories.created_at is TIMESTAMPTZ; decoding it as NaiveDateTime failed the whole
    // request the moment the tenant had a row (sqlx: TIMESTAMP is not compatible with TIMESTAMPTZ).
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list_categories(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let categories: Vec<CategoryRow> = sqlx::query_as(
        // FLEET-WIDE-CATEGORY-SCOPE (kanban t_a0214025): this route feeds the Affiliate Product
        // screen's Category select, so the rows it returns must be the rows a product can actually
        // CARRY. They come from a producer with no tenant filter at all —
        // `plan_handler::sync_plan_to_affiliate_product` resolves
        // `SELECT id FROM product_categories WHERE slug = 'funnelswift-plans' LIMIT 1`, which is the
        // SYSTEM tenant's row — and `list_affiliate_products` below already shows every caller the
        // system tenant's products (`ap.tenant_id = $1 OR ap.tenant_id = '…0001'`). A strict
        // `tenant_id = $1` filter therefore answered `[]` for a real admin (measured live) while the
        // products they edit carried a category, which would leave the select empty, unable to
        // represent the stored value, and make a save look like it clears it. The predicate below
        // mirrors the product list's own: the caller's categories plus the fleet's system-tenant ones.
        // Another REAL tenant's categories stay invisible, exactly as before.
        "SELECT id, tenant_id, name, slug, description, COALESCE(sort_order, 0) AS sort_order, COALESCE(is_active, true) AS is_active, created_at FROM product_categories WHERE (tenant_id = $1 OR tenant_id = '00000000-0000-0000-0000-000000000001') ORDER BY sort_order ASC, name ASC"
    )
    .bind(tenant_id)
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

    // product_categories.name / slug are NULLABLE with no default (8 of 15 live rows
    // carry a NULL slug): a non-Option decode 500'd the whole update on any such row.
    // Option + `.or()` keeps the stored value — NULL included — when the request does
    // not supply one, instead of inventing a value.
    let existing: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT name, slug, description FROM product_categories WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    let ex = existing.ok_or_else(|| AppError::NotFound("Category not found".into()))?;

    let name = req.name.or(ex.0);
    let slug = req.slug.or(ex.1);
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
