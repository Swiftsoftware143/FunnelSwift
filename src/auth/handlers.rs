use crate::email::send_reset_email;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize};
use serde_json::json;
use uuid::Uuid;
use std::env;
use crate::error::AppResult;
use crate::error::AppError;
use crate::auth::models::*;
use crate::auth::middleware::AuthUser;
use crate::state::AppState;

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    // Check if email already exists
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

    // Create tenant
    let tenant_id = Uuid::new_v4();
    let tenant_slug = req.tenant_name.to_lowercase().replace(' ', "-");
    sqlx::query(
        "INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)",
    )
    .bind(tenant_id)
    .bind(&req.tenant_name)
    .bind(&tenant_slug)
    .execute(&state.pool)
    .await?;

    // Create user
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&req.email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind("user")
    .execute(&state.pool)
    .await?;

    // Auto-generate API key for the user
    let api_key_id = Uuid::new_v4();
    let api_key_raw = format!("fs_{}_{}", 
        env::var("JWT_SECRET").unwrap_or_default().chars().take(4).collect::<String>(),
        Uuid::new_v4().to_string().replace("-", "").chars().take(24).collect::<String>()
    );
    let api_key_hash = format!("hash:{}", &api_key_raw);
    sqlx::query(
        "INSERT INTO api_keys (id, tenant_id, user_id, name, key_hash, prefix, permissions, full_key) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(api_key_id)
    .bind(tenant_id)
    .bind(user_id)
    .bind("Auto-generated")
    .bind(&api_key_hash)
    .bind(&api_key_raw.chars().take(8).collect::<String>())
    .bind(serde_json::json!(["read", "write"]))
    .bind(&api_key_raw)
    .execute(&state.pool)
    .await?;

    // Create default settings for tenant
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(serde_json::json!(["New", "Contacted", "Qualified", "Proposal", "Negotiation", "Closed Won", "Closed Lost"]))
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
                "tenant_id": tenant_id
            }
        })),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE email = $1 AND is_active = true",
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid email or password".into()))?;

    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|e| AppError::Internal(format!("Parse hash error: {e}")))?;

    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid email or password".into()))?;

    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user.id.to_string(),
        tenant_id: user.tenant_id.to_string(),
        email: user.email.clone(),
        role: user.role.clone(),
        exp: now + 86400 * 30,
        iat: now,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))?;

    Ok(Json(json!({
        "token": token,
        "user": {
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "role": user.role,
            "tenant_id": user.tenant_id
        }
    })))
}

pub async fn me(state: State<AppState>, auth: AuthUser) -> Json<serde_json::Value> {
    // Get user's name and username
    let user_info: Option<(String, Option<String>)> = 
        sqlx::query_as::<_, (String, Option<String>)>("SELECT name, username FROM users WHERE id::text = $1")
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);

    // Get user's API key
    let api_key_row: Option<(String, String, Option<String>)> = 
        match sqlx::query_as::<_, (String, String, Option<String>)>("SELECT prefix, name, full_key FROM api_keys WHERE user_id::text = $1 AND name = 'Auto-generated' LIMIT 1")
            .bind(&auth.user_id)
            .fetch_optional(&state.pool)
            .await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("API key query error: {:?}", e);
                    None
                }
            };

    // Get integration targets (affiliate products)
    let products: Vec<serde_json::Value> = 
        sqlx::query_as::<_, (serde_json::Value,)>(r#"SELECT row_to_json(t.*)::jsonb FROM target_software t ORDER BY t.name"#)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(r,)| r)
            .collect();

    // Get current plan subscription
    let current_plan: Option<serde_json::Value> =
        sqlx::query_as::<_, (serde_json::Value,)>(r#"
            SELECT row_to_json(p.*)::jsonb
            FROM plans p
            JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
            WHERE tps.tenant_id::text = $1 AND tps.status = 'active'
            ORDER BY tps.start_date DESC
            LIMIT 1
        "#)
            .bind(&auth.tenant_id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None)
            .map(|(r,)| r);

    Json(json!({
        "user_id": auth.user_id,
        "tenant_id": auth.tenant_id,
        "email": auth.email,
        "name": user_info.as_ref().map(|(n,_)| n.clone()).unwrap_or_default(),
        "username": user_info.as_ref().map(|(_,u)| u.clone()).flatten().unwrap_or_default(),
        "role": auth.role,
        "is_admin": auth.is_admin,
        "api_key": api_key_row.map(|(p, n, fk)| json!({"prefix": p, "name": n, "key": fk.unwrap_or_default()})),
        "available_products": products,
        "current_plan": current_plan
    }))
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub username: Option<String>,
}

pub async fn update_profile(
    state: State<AppState>,
    auth: AuthUser,
    Json(req): Json<UpdateProfileRequest>,
) -> Json<serde_json::Value> {
    sqlx::query("UPDATE users SET name = COALESCE($1, name), username = COALESCE($2, username) WHERE id::text = $3")
        .bind(&req.name)
        .bind(&req.username)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .unwrap_or_else(|_| panic!("Failed to update profile"));

    Json(json!({"status": "ok"}))
}

pub async fn change_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if req.new_password.len() < 8 {
        return Err(AppError::BadRequest("New password must be at least 8 characters".into()));
    }

    let user_id = uuid::Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user ID".into()))?;

    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE id = $1 AND is_active = true",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    // Verify current password
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|e| AppError::Internal(format!("Hash parse error: {e}")))?;
    Argon2::default()
        .verify_password(req.current_password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Current password is incorrect".into()))?;

    // Hash new password
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let new_hash = argon2
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();

    sqlx::query("UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2")
        .bind(&new_hash)
        .bind(user.id)
        .execute(&state.pool)
        .await?;

    Ok(Json(serde_json::json!({"message": "Password updated successfully"})))
}

pub async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if let Some(user) = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE email = $1 AND is_active = true",
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await?
    {
        let token = uuid::Uuid::new_v4().to_string();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);

        sqlx::query("UPDATE password_resets SET used = true WHERE user_id = $1 AND used = false")
            .bind(user.id)
            .execute(&state.pool)
            .await.ok();

        sqlx::query(
            "INSERT INTO password_resets (user_id, token, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(user.id)
        .bind(&token)
        .bind(expires_at)
        .execute(&state.pool)
        .await?;

        match send_reset_email(&user.email, &token).await {
            Ok(_) => tracing::info!("Password reset email sent to {}", user.email),
            Err(e) => tracing::error!("Failed to send password reset email to {}: {}", user.email, e),
        }
        // Send password reset email via SMTP
    }

    Ok(Json(serde_json::json!({"message": "If the email exists, a password reset link has been sent"})))
}

pub async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if req.new_password.len() < 8 {
        return Err(AppError::BadRequest("New password must be at least 8 characters".into()));
    }

    let reset = sqlx::query(
        "SELECT id, user_id, expires_at FROM password_resets WHERE token = $1 AND used = false AND expires_at > NOW()",
    )
    .bind(&req.token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".into()))?;

    use sqlx::Row;
    let reset_id: uuid::Uuid = reset.get("id");
    let user_id: uuid::Uuid = reset.get("user_id");

    // Hash new password
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let new_hash = argon2
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?;

    sqlx::query("UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2")
        .bind(new_hash.to_string())
        .bind(user_id)
        .execute(&state.pool)
        .await?;

    sqlx::query("UPDATE password_resets SET used = true WHERE id = $1")
        .bind(reset_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(serde_json::json!({"message": "Password has been reset successfully"})))
}

