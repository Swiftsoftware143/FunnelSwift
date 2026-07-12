use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo: Option<String>,
    pub colors: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
    pub created_at: chrono::NaiveDateTime,
    pub email: Option<String>,
    pub custom_fields: Option<serde_json::Value>,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub slug: Option<String>,
    pub email: Option<String>,
    pub plan_id: Option<Uuid>,
    pub initial_credits: Option<i64>,
    pub custom_fields: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub email: Option<String>,
    pub plan_id: Option<Uuid>,
    pub custom_fields: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AssignCreditsRequest {
    pub credits: Option<i64>,
    pub amount: Option<i64>,
    pub reason: Option<String>,
    pub transaction_type: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssignPlanRequest {
    pub plan_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct TenantWithMeta {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub email: Option<String>,
    pub logo: Option<String>,
    pub colors: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
    pub plan: Option<TenantPlanInfo>,
    pub credit_balance: i64,
    pub credits_used: i64,
    pub custom_fields: Option<serde_json::Value>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TenantPlanInfo {
    pub plan_id: Uuid,
    pub plan_name: String,
    pub status: String,
}

fn is_admin(auth: &AuthUser) -> bool {
    auth.role == "admin"
}

pub async fn list_tenants(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TenantWithMeta>>> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let tenants = sqlx::query_as::<_, Tenant>(
        "SELECT * FROM tenants ORDER BY name",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut result: Vec<TenantWithMeta> = Vec::new();
    for t in tenants {
        let plan = sqlx::query_as::<_, (Uuid, String, String)>(
            r#"SELECT p.id, p.name, tps.status
               FROM tenant_plan_subscriptions tps
               JOIN plans p ON p.id = tps.plan_id
               WHERE tps.tenant_id = $1 AND tps.status = 'active'
               LIMIT 1"#,
        )
        .bind(t.id)
        .fetch_optional(&state.pool)
        .await?
        .map(|(pid, pname, pstatus)| TenantPlanInfo {
            plan_id: pid,
            plan_name: pname,
            status: pstatus,
        });

        let credit_balance: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM credit_transactions WHERE tenant_id = $1",
        )
        .bind(t.id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

        let credits_used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(ABS(amount)), 0) FROM credit_transactions WHERE tenant_id = $1 AND amount < 0",
        )
        .bind(t.id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

        result.push(TenantWithMeta {
            id: t.id,
            name: t.name,
            slug: t.slug,
            logo: t.logo,
            colors: t.colors,
            settings: t.settings,
            email: t.email.clone(),
            credits_used,
            custom_fields: t.custom_fields.clone(),
            plan,
            credit_balance,
            created_at: t.created_at,
        });
    }

    Ok(Json(result))
}

pub async fn get_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<TenantWithMeta>> {
    let tenant_id = if is_admin(&auth) {
        id
    } else {
        auth.tenant_id.parse::<Uuid>().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?
    };

    let t = sqlx::query_as::<_, Tenant>(
        "SELECT * FROM tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tenant not found".into()))?;

    let plan = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT p.id, p.name, tps.status
           FROM tenant_plan_subscriptions tps
           JOIN plans p ON p.id = tps.plan_id
           WHERE tps.tenant_id = $1 AND tps.status = 'active'
           LIMIT 1"#,
    )
    .bind(t.id)
    .fetch_optional(&state.pool)
    .await?
    .map(|(pid, pname, pstatus)| TenantPlanInfo {
        plan_id: pid,
        plan_name: pname,
        status: pstatus,
    });

    let credit_balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0) FROM credit_transactions WHERE tenant_id = $1",
    )
    .bind(t.id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let credits_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(ABS(amount)), 0) FROM credit_transactions WHERE tenant_id = $1 AND amount < 0",
    )
    .bind(t.id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    Ok(Json(TenantWithMeta {
        id: t.id,
        name: t.name,
        slug: t.slug,
        email: t.email.clone(),
        logo: t.logo,
        colors: t.colors,
        settings: t.settings,
        plan,
        credit_balance,
        credits_used,
        custom_fields: t.custom_fields.clone(),
        created_at: t.created_at,
    }))
}

pub async fn create_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let tenant_id = Uuid::new_v4();
    let slug = req.slug.unwrap_or_else(|| req.name.to_lowercase().replace(" ", "-"));

    sqlx::query(
        "INSERT INTO tenants (id, name, slug, email, custom_fields) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&slug)
    .bind(&req.email)
    .bind(&req.custom_fields)
    .execute(&state.pool)
    .await?;

    // Assign plan if provided
    if let Some(plan_id) = req.plan_id {
        let sub_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status)
               VALUES ($1, $2, $3, 'active')"#,
        )
        .bind(sub_id)
        .bind(tenant_id)
        .bind(plan_id)
        .execute(&state.pool)
        .await?;
    }

    // Add initial credits if provided
    if let Some(credits) = req.initial_credits {
        if credits > 0 {
            let txn_id = Uuid::new_v4();
            sqlx::query(
                r#"INSERT INTO credit_transactions (id, tenant_id, amount, transaction_type, description)
                   VALUES ($1, $2, $3, 'admin_credit', 'Initial credits')"#,
            )
            .bind(txn_id)
            .bind(tenant_id)
            .bind(credits)
            .execute(&state.pool)
            .await?;
        }
    }

    Ok((StatusCode::CREATED, Json(json!({"id": tenant_id, "message": "Tenant created"}))))
}

pub async fn update_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTenantRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let existing = sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Tenant not found".into()))?;

    let name = req.name.unwrap_or(existing.name);
    let slug = req.slug.unwrap_or(existing.slug);

    sqlx::query("UPDATE tenants SET name=$1, slug=$2, email=$3, custom_fields=$4 WHERE id=$5")
        .bind(&name)
        .bind(&slug)
        .bind(&req.email)
        .bind(&req.custom_fields)
        .bind(id)
        .execute(&state.pool)
        .await?;

    // Update plan if provided
    if let Some(plan_id) = req.plan_id {
        // Deactivate existing
        sqlx::query(
            "UPDATE tenant_plan_subscriptions SET status = 'cancelled' WHERE tenant_id = $1 AND status = 'active'",
        )
        .bind(id)
        .execute(&state.pool)
        .await?;

        // Create new
        let sub_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status)
               VALUES ($1, $2, $3, 'active')"#,
        )
        .bind(sub_id)
        .bind(id)
        .bind(plan_id)
        .execute(&state.pool)
        .await?;
    }

    Ok(Json(json!({"message": "Tenant updated"})))
}

pub async fn delete_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Tenant deleted"})))
}

pub async fn get_tenant_credits(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0) FROM credit_transactions WHERE tenant_id = $1",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let transactions = sqlx::query_as::<_, (String, String, i64, chrono::NaiveDateTime)>(
        "SELECT transaction_type, description, amount, created_at FROM credit_transactions WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let txns: Vec<serde_json::Value> = transactions.into_iter().map(|(t, d, a, c)| {
        json!({"type": t, "description": d, "amount": a, "created_at": c})
    }).collect();

    Ok(Json(json!({
        "balance": balance,
        "transactions": txns
    })))
}

pub async fn assign_credits(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignCreditsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let amount = req.amount.or(req.credits).unwrap_or(0);
    if amount == 0 {
        return Err(AppError::BadRequest("Amount must be non-zero".into()));
    }
    let txn_type = req.transaction_type.unwrap_or_else(|| "admin_credit".to_string());
    let desc = req.description.or(req.reason).unwrap_or_else(|| "Admin credit".to_string());

    let txn_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO credit_transactions (id, tenant_id, amount, transaction_type, description)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(txn_id)
    .bind(id)
    .bind(amount)
    .bind(&txn_type)
    .bind(&desc)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"id": txn_id, "message": "Credits assigned"})))
}

pub async fn assign_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignPlanRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !is_admin(&auth) {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    // Deactivate existing subscriptions
    sqlx::query(
        "UPDATE tenant_plan_subscriptions SET status = 'cancelled' WHERE tenant_id = $1 AND status = 'active'",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;

    // Create new subscription
    let sub_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status)
           VALUES ($1, $2, $3, 'active')"#,
    )
    .bind(sub_id)
    .bind(id)
    .bind(req.plan_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Plan assigned successfully"})))
}
