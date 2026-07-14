use axum::{extract::{State, Json}, response::IntoResponse};
use serde_json::{json, Value};
use uuid::Uuid;
use jsonwebtoken::{encode, Header, EncodingKey};
use argon2::{Argon2, PasswordHasher};
use argon2::password_hash::SaltString;
use rand::rngs::OsRng;
use chrono::Utc;

use crate::error::{AppError, AppResult};
use crate::auth::models::Claims;
use crate::state::AppState;

/// Admin-only sync endpoint called by CoreSwift to create portfolio company with user account
pub async fn portfolio_sync(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<impl IntoResponse> {
    let sync_id = req.get("id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()).unwrap_or_else(Uuid::new_v4);
    let name = req.get("name").and_then(|v| v.as_str()).unwrap_or("Company").to_string();
    let email = req.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let description = req.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();

    if email.is_empty() {
        return Err(AppError::BadRequest("email is required for portfolio sync".into()));
    }

    // Check email uniqueness
    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    if existing > 0 {
        return Err(AppError::Conflict(format!("A user with email {} already exists", email)));
    }

    // Create tenant
    let tenant_id = Uuid::new_v4();
    let tenant_slug = name.to_lowercase().replace(' ', "-");

    sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind(&name)
        .bind(&tenant_slug)
        .execute(&state.pool)
        .await?;

    // Assign Enterprise plan
    let plan_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM plans WHERE name = 'Enterprise' LIMIT 1")
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

    if let Some(pid) = plan_id {
        sqlx::query(
            "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date) VALUES ($1, $2, $3, 'active', NOW())"
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(pid)
        .execute(&state.pool)
        .await
        .ok();
    }

    // Create user
    let user_id = Uuid::new_v4();
    let generated_password = Uuid::new_v4().to_string().replace("-", "").chars().take(12).collect::<String>();
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(generated_password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {}", e)))?
        .to_string();

    let now = Utc::now().naive_utc();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'member', true, $6, $7)"
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await?;

    // Create portfolio company record
    sqlx::query(
        "INSERT INTO portfolio_companies (id, tenant_id, name, slug, email, description, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW()) ON CONFLICT (id) DO UPDATE SET name = $3, email = $5, description = $6, updated_at = NOW()"
    )
    .bind(sync_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(&tenant_slug)
    .bind(&email)
    .bind(&description)
    .execute(&state.pool)
    .await?;

    // Sync to all other apps via internal endpoints
    let sync_body = json!({
        "tenant_id": tenant_id,
        "name": name,
        "slug": tenant_slug,
        "email": email,
        "description": description
    });
    let key = &state.internal_sync_key;
    let apps = [
        ("http://localhost:8087/api/v1/internal/portfolio-companies", "ADASwift"),
        ("http://localhost:8088/api/v1/internal/portfolio-companies", "MissedCall"),
        ("http://localhost:8085/api/v1/internal/portfolio-companies", "WorkflowSwift"),
        ("http://localhost:8084/api/portfolio/internal", "CoreSwift CRM"),
        ("http://localhost:8083/api/internal/portfolio-companies", "IncentiveSwift"),
    ];
    for (url, app_name) in apps {
        match reqwest::Client::new()
            .post(url)
            .header("x-internal-key", key)
            .header("Content-Type", "application/json")
            .json(&sync_body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    tracing::warn!("Portfolio sync to {} returned {}", app_name, status);
                }
            }
            Err(e) => {
                tracing::warn!("Portfolio sync to {} failed: {}", app_name, e);
            }
        }
    }

    Ok(Json(json!({
        "status": "synced",
        "id": sync_id.to_string(),
        "name": name,
        "email": email,
        "tenant_id": tenant_id.to_string(),
        "user_id": user_id.to_string(),
        "password": generated_password,
        "note": "Share credentials with the company. They can login directly."
    })))
}

/// Admin impersonation — generates a FunnelSwift-compatible JWT for the target tenant
pub async fn impersonate(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<impl IntoResponse> {
    let target_tenant_id = req.get("tenant_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("tenant_id is required".into()))?;

    let now = Utc::now().timestamp() as usize;
    let imp_claims = Claims {
        sub: Uuid::new_v4().to_string(),
        tenant_id: target_tenant_id.to_string(),
        email: format!("impersonated@{}", target_tenant_id),
        role: "impersonated".to_string(),
        exp: now + 900,
        iat: now,
    };

    let token = encode(
        &Header::default(),
        &imp_claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {}", e)))?;

    Ok(Json(json!({
        "impersonation_token": token,
        "expires_in": 900,
        "token_type": "Bearer",
        "message": "Full tenant switch. Admin panel disappears."
    })))
}

/// Stop impersonation
pub async fn stop_impersonation() -> AppResult<impl IntoResponse> {
    Ok(Json(json!({
        "status": "impersonation_stopped",
        "note": "Drop impersonation token. Restore your original admin token."
    })))
}
