// Affiliate product handler - full CRUD with admin variants
// David's model: affiliate products = the Swift products (CoreSwift, FunnelSwift,
// IncentiveSwift, MultiDirectory, etc.). Admin adds products and assigns a system
// tag to each. When a lead gets that system tag, the affiliate system records attribution.
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
pub struct CreateProductRequest {
    pub name: String,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub category_id: Option<Uuid>,
    pub url: Option<String>,
    pub is_third_party: Option<bool>,
    pub product_type: Option<String>,
    pub owner_name: Option<String>,
    pub system_tag_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub is_active: Option<bool>,
    pub category_id: Option<Uuid>,
    pub url: Option<String>,
    pub is_third_party: Option<bool>,
    pub product_type: Option<String>,
    pub owner_name: Option<String>,
    pub system_tag_id: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct AffiliateProductRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub is_active: bool,
    pub is_third_party: Option<bool>,
    pub url: Option<String>,
    pub category_id: Option<Uuid>,
    pub product_type: Option<String>,
    pub owner_name: Option<String>,
    pub system_tag_id: Option<Uuid>,
    pub system_tag_name: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

pub async fn list_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let products: Vec<AffiliateProductRow> = sqlx::query_as(
        "SELECT ap.id, ap.tenant_id, ap.name, ap.description, ap.price::float8, ap.default_commission_rate::float8,
                ap.is_active, ap.is_third_party, ap.url, ap.category_id,
                ap.product_type, ap.owner_name, ap.system_tag_id,
                t.name AS system_tag_name, ap.created_at::timestamp, ap.updated_at::timestamp
         FROM affiliate_products ap
         LEFT JOIN tags t ON t.id = ap.system_tag_id
         WHERE (ap.tenant_id = $1 OR ap.tenant_id = '00000000-0000-0000-0000-000000000001')
         ORDER BY ap.created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let result: Vec<Value> = products
        .iter()
        .map(|p| {
            json!({
                "id": p.id.to_string(),
                "name": p.name,
                "description": p.description,
                "price": p.price.unwrap_or(0.0),
                "default_commission_rate": p.default_commission_rate.unwrap_or(0.0),
                "is_active": p.is_active,
                "is_third_party": p.is_third_party.unwrap_or(false),
                "url": p.url,
                "category_id": p.category_id.map(|v| v.to_string()),
                "product_type": p.product_type.as_deref().unwrap_or("software"),
                "owner_name": p.owner_name.as_deref().unwrap_or("SwiftSoftware"),
                "system_tag_id": p.system_tag_id.map(|v| v.to_string()),
                "system_tag_name": p.system_tag_name,
                "created_at": p.created_at,
                "updated_at": p.updated_at,
            })
        })
        .collect();

    Ok(Json(json!(result)))
}

pub async fn list_all_affiliate_products_admin(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    list_affiliate_products(auth, State(state)).await
}

/// List system tags available for assignment to affiliate products (admin dropdown).
pub async fn list_system_tags(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tags: Vec<(Uuid, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, color FROM tags WHERE is_system = true ORDER BY name")
            .fetch_all(&state.pool)
            .await?;
    let result: Vec<Value> = tags
        .iter()
        .map(|(id, name, color)| json!({ "id": id.to_string(), "name": name, "color": color }))
        .collect();
    Ok(Json(json!(result)))
}

pub async fn create_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateProductRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let id = Uuid::new_v4();
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query(
        "INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, url, category_id, product_type, owner_name, system_tag_id)
         VALUES ($1, $2, $3, $4, $5, $6, true, $7, $8, $9, $10, $11, $12)"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price.unwrap_or(0.0))
    .bind(req.default_commission_rate.unwrap_or(0.0))
    .bind(req.is_third_party.unwrap_or(false))
    .bind(&req.url)
    .bind(req.category_id)
    .bind(req.product_type.unwrap_or_else(|| "software".to_string()))
    .bind(req.owner_name.unwrap_or_else(|| "SwiftSoftware".to_string()))
    .bind(req.system_tag_id)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.to_string(), "message": "Product created"})),
    ))
}

pub async fn update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProductRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_as::<_, (String, Option<String>, f64, f64, bool, Option<String>, Option<Uuid>, Option<String>, Option<String>, Option<Uuid>)>(
        "SELECT name, description, COALESCE(price,0.0), COALESCE(default_commission_rate,0.0), COALESCE(is_third_party,false), url, category_id, product_type, owner_name, system_tag_id
         FROM affiliate_products WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Product not found".into()))?;

    let name = req.name.unwrap_or(existing.0);
    let description = req.description.or(existing.1);
    let price = req.price.unwrap_or(existing.2);
    let commission = req.default_commission_rate.unwrap_or(existing.3);
    let is_third_party = req.is_third_party.unwrap_or(existing.4);
    let url = req.url.or(existing.5);
    let category_id = req.category_id.or(existing.6);
    let product_type = req
        .product_type
        .unwrap_or(existing.7.unwrap_or_else(|| "software".to_string()));
    let owner_name = req
        .owner_name
        .unwrap_or(existing.8.unwrap_or_else(|| "SwiftSoftware".to_string()));
    // system_tag_id: explicit Some(null) clears the tag, None leaves it unchanged.
    let system_tag_id = match req.system_tag_id {
        Some(v) => Some(v),
        None => existing.9,
    };

    sqlx::query(
        "UPDATE affiliate_products SET name=$1, description=$2, price=$3, default_commission_rate=$4,
         is_third_party=$5, url=$6, category_id=$7, product_type=$8, owner_name=$9, system_tag_id=$10, updated_at=NOW()
         WHERE id=$11 AND tenant_id=$12"
    )
    .bind(&name)
    .bind(&description)
    .bind(price)
    .bind(commission)
    .bind(is_third_party)
    .bind(&url)
    .bind(category_id)
    .bind(&product_type)
    .bind(&owner_name)
    .bind(system_tag_id)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Product updated"})))
}

pub async fn delete_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("DELETE FROM affiliate_products WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Product deleted"})))
}

pub async fn admin_sync_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plans: Vec<(Uuid, String, Option<f64>)> =
        sqlx::query_as("SELECT id, name, price FROM plans LIMIT 50")
            .fetch_all(&state.pool)
            .await?;

    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let mut count: i64 = 0;

    for (plan_id, plan_name, plan_price) in plans {
        let exists: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM affiliate_products WHERE plan_id = $1")
                .bind(plan_id)
                .fetch_one(&state.pool)
                .await?;

        if exists.0 == 0 {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO affiliate_products (id, tenant_id, name, price, plan_id, product_type, owner_name, default_commission_rate)
                 VALUES ($1, $2, $3, $4, $5, 'software', 'SwiftSoftware', 10.0)"
            )
            .bind(id)
            .bind(tenant_id)
            .bind(&plan_name)
            .bind(plan_price)
            .bind(plan_id)
            .execute(&state.pool)
            .await?;
            count += 1;
        }
    }

    Ok(Json(
        json!({"synced": count, "message": format!("{} products synced", count)}),
    ))
}

pub async fn admin_update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProductRequest>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    update_affiliate_product(auth, State(state), Path(id), Json(req)).await
}

pub async fn handle_cross_app_plan_sync(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    // Internal endpoint — verify x-internal-key before doing anything.
    let key = headers
        .get("x-internal-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if key != state.internal_sync_key {
        return Err(AppError::Forbidden("Invalid internal key".into()));
    }

    let plan_name = payload["plan_name"].as_str().unwrap_or("Unknown Plan");
    let plan_price = payload["price"].as_f64().unwrap_or(0.0);
    let plan_id_str = payload["plan_id"].as_str().unwrap_or("");
    let source_app = payload["source_app"].as_str().unwrap_or("unknown");

    let plan_id = Uuid::parse_str(plan_id_str).ok();
    let tenant_id = payload["tenant_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO affiliate_products (id, tenant_id, name, price, plan_id, source_app, product_type, owner_name, default_commission_rate)
         VALUES ($1, $2, $3, $4, $5, $6, 'software', 'SwiftSoftware', 10.0)"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(plan_name)
    .bind(plan_price)
    .bind(plan_id)
    .bind(source_app)
    .execute(&state.pool)
    .await?;

    Ok(Json(
        json!({"status": "synced", "product_id": id.to_string()}),
    ))
}
