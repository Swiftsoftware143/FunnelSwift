use crate::error::{AppError, AppResult};
use crate::state::AppState;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::models::Claims;

/// POST /api/v1/auth/signup — Public signup (used by Kinetic Cards landing page).
/// Creates tenant + user, assigns plan from request body (defaults to 'free').
pub async fn public_signup(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let email = payload["email"].as_str().unwrap_or("").trim().to_string();
    let password = payload["password"].as_str().unwrap_or("").to_string();
    let name = payload["name"].as_str().unwrap_or("").trim().to_string();
    // Always start on the free tier — never honour a caller-supplied plan slug.
    let plan_slug = "kinetic-free".to_string();
    let source = payload["source"].as_str().unwrap_or("").to_string();
    let affiliate_code = payload["affiliate_code"].as_str().map(|s| s.to_string());

    if email.is_empty() || password.is_empty() || name.is_empty() {
        return Err(AppError::BadRequest(
            "Name, email, and password are required".into(),
        ));
    }
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if !email.contains('@') {
        return Err(AppError::BadRequest("Invalid email format".into()));
    }

    // Check for duplicate email
    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE email = $1")
        .bind(&email)
        .fetch_one(&state.pool)
        .await?;

    if existing > 0 {
        return Err(AppError::Conflict(
            "An account with this email already exists".into(),
        ));
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();

    // Create tenant
    let tenant_id = Uuid::new_v4();
    let tenant_slug = format!(
        "{}-{}",
        name.to_lowercase().replace(' ', "-"),
        Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind(format!("{}'s Workspace", name))
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
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .bind("user")
    .execute(&state.pool)
    .await?;

    // Create default lead stages
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(json!(["New", "Contacted", "Qualified", "Proposal", "Negotiation", "Closed Won", "Closed Lost"]))
    .execute(&state.pool)
    .await?;

    // Assign plan (respects plan from signup request, defaults to 'free')
    let plan_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM plans WHERE slug = $1 LIMIT 1")
        .bind(&plan_slug)
        .fetch_optional(&state.pool)
        .await?;
    if let Some(pid) = plan_id {
        let _ = sqlx::query(
            r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date)
               VALUES ($1, $2, $3, 'active', NOW())"#,
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(pid)
        .execute(&state.pool)
        .await;
    }

    // Log source if provided
    if !source.is_empty() {
        tracing::info!(
            source = %source,
            email = %email,
            plan = %plan_slug,
            "Public signup completed"
        );
    }

    // Handle affiliate referral code
    if let Some(ref_code) = affiliate_code {
        if !ref_code.is_empty() {
            let _ = sqlx::query(
                r#"INSERT INTO referral_tracking (id, referrer_code, referred_email, referred_tenant_id, created_at)
                   VALUES ($1, $2, $3, $4, NOW())"#
            )
            .bind(Uuid::new_v4())
            .bind(&ref_code)
            .bind(&email)
            .bind(tenant_id)
            .execute(&state.pool)
            .await;
        }
    }

    // Mint the same session JWT the main register path returns. Without it the Kinetic
    // landing page had nothing to hand the app: it stored `undefined` as the token and
    // then redirected to /dashboard on the *marketing* host, which is a 404. The app runs
    // on its own origin, so the caller passes this token over as ?token= (exactly what
    // the main landing page's register flow does); the SPA stores it and strips the URL.
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        email: email.clone(),
        role: "user".into(),
        exp: now + 86400 * 30,
        iat: now,
        aud: Some("funnelswift-api".to_string()),
        iss: Some("funnelswift".to_string()),
        impersonating: None,
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
            "message": "Account created successfully",
            "token": token,
            "plan": plan_slug,
            "user": {
                "id": user_id,
                "email": email,
                "name": name,
                "role": "user",
                "tenant_id": tenant_id
            }
        })),
    ))
}
