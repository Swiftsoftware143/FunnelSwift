use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::auth::middleware::AuthUser;
use crate::models::plan::*;
use crate::state::AppState;
use crate::tag_logic;
use sqlx::Row;

pub async fn list_plans(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Plan>>> {
    let plans = sqlx::query_as::<_, Plan>("SELECT * FROM plans ORDER BY price")
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(plans))
}

pub async fn create_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePlanRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plan_id = Uuid::new_v4();

    sqlx::query(
        r#"INSERT INTO plans (id, name, slug, price, purchase_url, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label, payment_provider, features)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(plan_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(req.price)
    .bind(&req.purchase_url)
    .bind(req.max_leads)
    .bind(req.max_tags)
    .bind(req.has_dual_routing.unwrap_or(false))
    .bind(req.has_multi_tenant.unwrap_or(false))
    .bind(req.has_white_label.unwrap_or(false))
    .bind(&req.payment_provider)
    .bind(&req.features)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": plan_id, "message": "Plan created"}))))
}

pub async fn get_plan(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Plan>> {
    let plan = sqlx::query_as::<_, Plan>("SELECT * FROM plans WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    Ok(Json(plan))
}

pub async fn update_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePlanRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let existing = sqlx::query_as::<_, Plan>("SELECT * FROM plans WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    sqlx::query(
        r#"UPDATE plans SET name=$1, slug=$2, price=$3, purchase_url=$4, max_leads=$5, max_tags=$6,
           has_dual_routing=$7, has_multi_tenant=$8, has_white_label=$9, payment_provider=$10, features=$11, updated_at=NOW()
           WHERE id=$12"#,
    )
    .bind(req.name.unwrap_or(existing.name))
    .bind(req.slug.unwrap_or(existing.slug))
    .bind(req.price.unwrap_or(existing.price))
    .bind(&req.purchase_url)
    .bind(req.max_leads.or(existing.max_leads))
    .bind(req.max_tags.or(existing.max_tags))
    .bind(req.has_dual_routing.unwrap_or(existing.has_dual_routing))
    .bind(req.has_multi_tenant.unwrap_or(existing.has_multi_tenant))
    .bind(req.has_white_label.unwrap_or(existing.has_white_label))
    .bind(&req.payment_provider.or(existing.payment_provider))
    .bind(req.features.or(existing.features))
    .bind(id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Plan updated"})))
}

pub async fn delete_plan_admin(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let result = sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Plan not found".into()));
    }

    Ok(Json(json!({"message": "Plan deleted"})))
}


// ── Admin endpoints (follow funnelswift pattern - no auth extractor) ──

pub async fn admin_list_all_plans(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plans = sqlx::query("SELECT id, name, slug, price, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label, payment_provider, features, created_at, updated_at FROM plans ORDER BY price")
        .fetch_all(&state.pool)
        .await?;

    let result: Vec<serde_json::Value> = plans.iter().map(|row| {
        json!({
            "id": row.try_get::<Uuid, _>("id").map(|u| u.to_string()).unwrap_or_default(),
            "name": row.try_get::<String, _>("name").unwrap_or_default(),
            "slug": row.try_get::<String, _>("slug").unwrap_or_default(),
            "price": row.try_get::<f64, _>("price").unwrap_or(0.0),
            "max_leads": row.try_get::<Option<i32>, _>("max_leads").unwrap_or(None),
            "max_tags": row.try_get::<Option<i32>, _>("max_tags").unwrap_or(None),
            "payment_provider": row.try_get::<Option<String>, _>("payment_provider").unwrap_or(None),
            "has_dual_routing": row.try_get::<bool, _>("has_dual_routing").unwrap_or(false),
            "has_multi_tenant": row.try_get::<bool, _>("has_multi_tenant").unwrap_or(false),
            "has_white_label": row.try_get::<bool, _>("has_white_label").unwrap_or(false),
            "features": row.try_get::<Option<serde_json::Value>, _>("features").unwrap_or(None),
        })
    }).collect();

    Ok(Json(json!({"plans": result, "total": result.len()})))
}

pub async fn admin_create_plan_json(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plan_id = Uuid::new_v4();
    let name = req.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let slug = req.get("slug").and_then(|v| v.as_str()).unwrap_or(&name.to_lowercase().replace(" ", "-")).to_string();
    let price = req.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let max_leads = req.get("max_leads").and_then(|v| v.as_i64()).map(|v| v as i32);
    let max_tags = req.get("max_tags").and_then(|v| v.as_i64()).map(|v| v as i32);
    let has_dual_routing = req.get("has_dual_routing").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_multi_tenant = req.get("has_multi_tenant").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_white_label = req.get("has_white_label").and_then(|v| v.as_bool()).unwrap_or(false);
    let purchase_url = req.get("purchase_url").and_then(|v| v.as_str()).map(|s| s.to_string());
    let payment_provider = req.get("payment_provider").and_then(|v| v.as_str()).map(|s| s.to_string());
    let features = req.get("features").cloned();

    if name.is_empty() {
        return Err(AppError::BadRequest("Plan name is required".into()));
    }

    sqlx::query(
        r#"INSERT INTO plans (id, name, slug, price, purchase_url, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label, payment_provider, features)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(plan_id)
    .bind(&name)
    .bind(&slug)
    .bind(price)
    .bind(&purchase_url)
    .bind(max_leads)
    .bind(max_tags)
    .bind(has_dual_routing)
    .bind(has_multi_tenant)
    .bind(has_white_label)
    .bind(&payment_provider)
    .bind(&features)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": plan_id, "message": "Plan created"}))))
}

pub async fn admin_update_plan_features(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let features = req.get("features")
        .ok_or_else(|| AppError::BadRequest("features object is required".to_string()))?;
    let features_str = features.to_string();

    sqlx::query(
        "UPDATE plans SET features = features::jsonb || $1::jsonb, updated_at = NOW() WHERE id = $2",
    )
    .bind(&features_str)
    .bind(id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Features updated"})))
}

pub async fn admin_assign_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id = req.get("tenant_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("Valid tenant_id is required".into()))?;
    let plan_id = req.get("plan_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("Valid plan_id is required".into()))?;

    // Get the new plan slug before activating
    let new_plan_slug: String = sqlx::query_scalar(
        "SELECT slug FROM plans WHERE id = $1"
    )
    .bind(plan_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    // Deactivate existing subscription first
    sqlx::query(
        "UPDATE tenant_plan_subscriptions SET status = 'cancelled' WHERE tenant_id = $1 AND status = 'active'"
    )
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date)
           VALUES ($1, $2, $3, 'active', NOW())"#,
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(plan_id)
    .execute(&state.pool)
    .await?;

    // Auto-apply Sold tag to all leads in this tenant
    // This only applies when upgrading to paid plans (pro/enterprise)
    tag_logic::apply_sold_to_tenant_leads(
        &state.pool,
        tenant_id,
        &new_plan_slug,
    ).await?;

    Ok(Json(json!({"message": "Plan assigned to tenant", "subscription_id": subscription_id})))
}
