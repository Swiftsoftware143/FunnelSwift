use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Incoming Mintbird webhook payload.
#[derive(Debug, Deserialize)]
pub struct MintbirdWebhook {
    pub event: String,
    pub data: MintbirdPurchaseData,
}

#[derive(Debug, Deserialize)]
pub struct MintbirdPurchaseData {
    pub plan_slug: String,
    pub customer_email: String,
    pub customer_name: Option<String>,
    pub transaction_id: Option<String>,
}

/// POST /api/v1/webhooks/mintbird — handle purchase.completed from Mintbird.
pub async fn handle_purchase(
    State(state): State<AppState>,
    Json(payload): Json<MintbirdWebhook>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let plan_slug = payload.data.plan_slug.clone();
    let email = payload.data.customer_email.clone();

    // 1. Look up plan by slug
    let plan_row = sqlx::query(
        "SELECT id, name, slug, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label FROM plans WHERE slug = $1",
    )
    .bind(&plan_slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Plan with slug '{}' not found", plan_slug)))?;

    use sqlx::Row;
    let plan_id: Uuid = plan_row.get("id");
    let plan_name: String = plan_row.get("name");

    // 2. Find or create tenant by email
    let tenant_slug = email.replace('@', "-at-").replace('.', "-dot-").to_lowercase();
    let tenant_name = payload.data.customer_name.clone().unwrap_or_else(|| email.clone());

    let tenant_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM tenants WHERE slug = $1",
    )
    .bind(&tenant_slug)
    .fetch_optional(&state.pool)
    .await?
    {
        Some(id) => id,
        None => {
            let new_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)",
            )
            .bind(new_id)
            .bind(&tenant_name)
            .bind(&tenant_slug)
            .execute(&state.pool)
            .await?;
            new_id
        }
    };

    // 3. Ensure a user exists for this email
    let user_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE tenant_id = $1 AND email = $2",
    )
    .bind(tenant_id)
    .bind(&email)
    .fetch_one(&state.pool)
    .await?
        > 0;

    if !user_exists {
        let user_id = Uuid::new_v4();
        // Create user with empty password hash (they will reset via forgot-password)
        sqlx::query(
            r#"INSERT INTO users (id, tenant_id, email, password_hash, name, role)
               VALUES ($1, $2, $3, '', $4, 'user')"#,
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(&email)
        .bind(&tenant_name)
        .execute(&state.pool)
        .await?;
    }

    // 4. Assign plan to tenant (upsert)
    let subscription_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date)
           VALUES ($1, $2, $3, 'active', NOW())
           ON CONFLICT (tenant_id, plan_id) DO UPDATE SET status = 'active', start_date = NOW()"#,
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(plan_id)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        "Mintbird: purchase completed — plan={}, email={}, tenant={}",
        plan_slug, email, tenant_id
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "plan": plan_name,
            "tenant_id": tenant_id.to_string(),
            "email": email,
        })),
    ))
}
