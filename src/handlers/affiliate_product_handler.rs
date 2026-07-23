use axum::{extract::{Path, State}, Json};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::models::affiliate_product::*;
use crate::error::AppError;
use crate::error::AppResult;
use crate::AppState;
use serde_json::json;
use serde::Deserialize;
use axum::http::StatusCode;

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

// ── Admin endpoints ──

#[derive(Debug, Deserialize)]
pub struct AdminUpdateAffiliateProductRequest {
    pub owner_name: Option<String>,
    pub product_type: Option<String>,
}

/// PUT /api/v1/admin/affiliate-products/:id
/// Allows super_admin to edit owner_name and product_type on any affiliate product.
pub async fn admin_update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AdminUpdateAffiliateProductRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if auth.role != "super_admin" {
        return Err(AppError::Forbidden("Super admin access required".into()));
    }

    // Validate product_type if provided
    if let Some(ref pt) = req.product_type {
        if pt != "software" && pt != "service" {
            return Err(AppError::BadRequest("product_type must be 'software' or 'service'".into()));
        }
    }

    let product = sqlx::query_as::<_, (serde_json::Value,)>(r#"
        UPDATE affiliate_products SET
            owner_name = COALESCE($2, owner_name),
            product_type = COALESCE($3, product_type),
            updated_at = NOW()
        WHERE id = $1
        RETURNING row_to_json(affiliate_products.*)::jsonb
    "#)
    .bind(id)
    .bind(&req.owner_name)
    .bind(&req.product_type)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| AppError::NotFound("Affiliate product not found".into()))?;

    Ok(Json(json!({"data": product})))
}

/// POST /api/v1/admin/affiliate-products/sync
/// Manually triggers a full sync of all plans → affiliate_products.
/// Useful for initial setup or re-sync after schema changes.
pub async fn admin_sync_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    // Fetch all active plans
    let plans = sqlx::query_as::<_, (Uuid, String, f64,)>(
        "SELECT id, name, price FROM plans ORDER BY price"
    )
    .fetch_all(&state.pool)
    .await?;

    // Get the FunnelSwift Plans category
    let category_id: Option<Uuid> = match sqlx::query_scalar(
        "SELECT id FROM product_categories WHERE slug = 'funnelswift-plans' LIMIT 1"
    )
    .fetch_optional(&state.pool)
    .await?
    {
        Some(id) => Some(id),
        None => {
            // Fallback to any category
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM product_categories LIMIT 1")
                .fetch_optional(&state.pool)
                .await?
        }
    };

    // Get default tenant
    let default_tenant: Uuid = sqlx::query_scalar(
        "SELECT id FROM tenants ORDER BY created_at LIMIT 1"
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_else(Uuid::nil);

    let mut synced = 0u64;

    for (plan_id, name, price) in &plans {
        // Check if already synced
        let existing: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM affiliate_products WHERE plan_id = $1"
        )
        .bind(plan_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

        if existing > 0 {
            // Update existing
            let description = format!("{} — FunnelSwift Plan", name);
            sqlx::query(
                r#"UPDATE affiliate_products SET
                    name = $1,
                    description = $2,
                    price = $3,
                    is_active = true,
                    updated_at = NOW()
                WHERE plan_id = $4"#
            )
            .bind(name)
            .bind(&description)
            .bind(price)
            .bind(plan_id)
            .execute(&state.pool)
            .await?;
            synced += 1;
        } else {
            // Insert new
            let description = format!("{} — FunnelSwift Plan", name);
            let default_commission: f64 = 20.0;
            sqlx::query(
                r#"INSERT INTO affiliate_products
                    (tenant_id, name, description, price, default_commission_rate, is_active, category_id, plan_id, owner_name, product_type, source_app)
                VALUES ($1, $2, $3, $4, $5, true, $6, $7, 'SwiftSoftware', 'software', 'funnelswift')"#
            )
            .bind(default_tenant)
            .bind(name)
            .bind(&description)
            .bind(price)
            .bind(default_commission)
            .bind(category_id)
            .bind(plan_id)
            .execute(&state.pool)
            .await?;
            synced += 1;
        }
    }

    Ok(Json(json!({
        "message": "Affiliate products synced",
        "synced": synced,
        "total_plans": plans.len()
    })))
}

/// Cross-app webhook: receive plan sync events from other apps (IncentiveSwift, etc.)
/// and create/update/deactivate the corresponding affiliate product in FunnelSwift.
pub async fn handle_cross_app_plan_sync(
    State(state): State<crate::AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let plan_name = body.get("plan_name").and_then(|v| v.as_str()).unwrap_or("Unknown Plan");
    let plan_price = body.get("plan_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let source_app = body.get("source_app").and_then(|v| v.as_str()).unwrap_or("external");
    let is_active = body.get("is_active").and_then(|v| v.as_bool()).unwrap_or(true);
    let owner_name = body.get("owner_name").and_then(|v| v.as_str()).unwrap_or("SwiftSoftware");
    let product_type = body.get("product_type").and_then(|v| v.as_str()).unwrap_or("software");

    let category_slug = format!("{}-plans", source_app);
    let category_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM product_categories WHERE slug = $1"
    )
    .bind(&category_slug)
    .fetch_optional(&state.pool)
    .await?;

    let category_id = match category_id {
        Some(id) => id,
        None => return Err(AppError::BadRequest(format!(
            "No product category for '{}' — run migrations", source_app
        ))),
    };

    let tenant_id: Uuid = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM tenants ORDER BY created_at ASC LIMIT 1"
    )
    .fetch_optional(&state.pool)
    .await?
    {
        Some(t) => t,
        None => return Err(AppError::Internal("No tenants found".into())),
    };

    match action {
        "create" | "update" => {
            let existing: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM affiliate_products WHERE source_app = $1 AND name = $2"
            )
            .bind(source_app)
            .bind(plan_name)
            .fetch_optional(&state.pool)
            .await?;

            if let Some(aff_id) = existing {
                sqlx::query(
                    "UPDATE affiliate_products SET name = $1, price = $2, is_active = $3, updated_at = NOW() WHERE id = $4"
                )
                .bind(plan_name)
                .bind(plan_price)
                .bind(is_active)
                .bind(aff_id)
                .execute(&state.pool)
                .await?;
            } else {
                sqlx::query(
                    r#"INSERT INTO affiliate_products (tenant_id, name, description, price, default_commission_rate, is_active, category_id, owner_name, product_type, source_app)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#
                )
                .bind(tenant_id)
                .bind(plan_name)
                .bind(format!("{} plan from {}", plan_name, source_app))
                .bind(plan_price)
                .bind(20.0)
                .bind(true)
                .bind(category_id)
                .bind(owner_name)
                .bind(product_type)
                .bind(source_app)
                .execute(&state.pool)
                .await?;
            }
        }
        "deactivate" => {
            sqlx::query(
                "UPDATE affiliate_products SET is_active = false, updated_at = NOW() WHERE source_app = $1 AND name = $2"
            )
            .bind(source_app)
            .bind(plan_name)
            .execute(&state.pool)
            .await?;
        }
        _ => {
            return Err(AppError::BadRequest(format!("Unknown action: {}", action)));
        }
    }

    Ok(Json(json!({"status": "synced", "action": action, "plan": plan_name, "source_app": source_app})))
}
