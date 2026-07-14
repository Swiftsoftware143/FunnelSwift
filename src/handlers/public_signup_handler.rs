use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use std::env;
use crate::auth::models::Claims;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PublicSignupRequest {
    pub email: String,
    pub password: String,
    pub name: String,
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "homepage".to_string()
}

/// POST /api/v1/auth/signup — lightweight signup for public homepage
/// Auto-generates tenant name. No tenant_name field needed.
/// Accepts optional `source` field for tracking signup origin.
pub async fn public_signup(
    State(state): State<AppState>,
    Json(req): Json<PublicSignupRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    // Validate
    if req.password.len() < 6 {
        return Err(AppError::BadRequest("Password must be at least 6 characters".into()));
    }
    if req.email.is_empty() || !req.email.contains('@') {
        return Err(AppError::BadRequest("Valid email required".into()));
    }
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Name is required".into()));
    }

    // Normalize source
    let source = if req.source.trim().is_empty() {
        "homepage".to_string()
    } else {
        req.source.trim().to_lowercase()
    };

    // Check existing
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE email = $1",
    )
    .bind(&req.email)
    .fetch_one(&state.pool)
    .await?;

    if existing > 0 {
        return Err(AppError::Conflict("A user with this email already exists. Try signing in.".into()));
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();

    // Auto-generate tenant name from user name
    let tenant_name = format!("{}'s Workspace", req.name);
    let tenant_slug = format!("ws-{}", Uuid::new_v4().to_string().split('-').next().unwrap_or("user"));

    // Create tenant
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)",
    )
    .bind(tenant_id)
    .bind(&tenant_name)
    .bind(&tenant_slug)
    .execute(&state.pool)
    .await?;

    // Create user with source_of_entry
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, source_of_entry) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&req.email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind("user")
    .bind(&source)
    .execute(&state.pool)
    .await?;

    // Default settings
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(json!(["New", "Contacted", "Qualified", "Proposal", "Negotiation", "Closed Won", "Closed Lost"]))
    .execute(&state.pool)
    .await?;

    // Assign Free plan
    let free_plan_id = uuid::Uuid::parse_str("f0000000-0000-0000-0000-000000000001").expect("Free plan UUID");
    sqlx::query(
        "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status) VALUES ($1, $2, $3, 'active')"
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(free_plan_id)
    .execute(&state.pool)
    .await?;

    // Generate JWT
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        email: req.email.clone(),
        role: "user".into(),
        exp: now + 86400 * 30,
        iat: now,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "token": token,
            "user": {
                "id": user_id,
                "email": req.email,
                "name": req.name,
                "role": "user",
                "tenant_id": tenant_id,
                "source": source
            },
            "tenant": {
                "id": tenant_id,
                "name": tenant_name
            },
            "redirect": "/dashboard"
        })),
    ))
}
