use crate::email::send_reset_email;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::*;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
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
        return Err(AppError::Conflict("Email already registered".into()));
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
    .bind("admin")
    .execute(&state.pool)
    .await?;

    // Create default settings for tenant
    sqlx::query(
        "INSERT INTO tenant_settings (tenant_id, key, value) VALUES ($1, 'lead_stages', $2)",
    )
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
        role: "admin".into(),
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
                "role": "admin",
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

pub async fn me(auth: AuthUser) -> Json<serde_json::Value> {
    Json(json!({
        "user_id": auth.user_id,
        "tenant_id": auth.tenant_id,
        "email": auth.email,
        "role": auth.role
    }))
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
        .bind(&new_hash.to_string())
        .bind(user_id)
        .execute(&state.pool)
        .await?;

    sqlx::query("UPDATE password_resets SET used = true WHERE id = $1")
        .bind(reset_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(serde_json::json!({"message": "Password has been reset successfully"})))
}

