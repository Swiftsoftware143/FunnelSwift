use crate::error::{AppError, AppResult};
use crate::state::AppState;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;

/// POST /api/v1/auth/signup — Public signup (used by Kinetic Cards landing page).
/// Creates tenant + user, assigns plan from request body (defaults to 'free').
pub async fn public_signup(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let email = payload["email"].as_str().unwrap_or("").trim().to_string();
    let password = payload["password"].as_str().unwrap_or("").to_string();
    let name = payload["name"].as_str().unwrap_or("").trim().to_string();
    let plan_slug = payload["plan"]
        .as_str()
        .unwrap_or("kinetic-free")
        .to_string();
    let source = payload["source"].as_str().unwrap_or("").to_string();
    let affiliate_code = payload["affiliate_code"].as_str().map(|s| s.to_string());

    if email.is_empty() || password.is_empty() || name.is_empty() {
        return Err(AppError::BadRequest(
            "Name, email, and password are required".into(),
        ));
    }
    if password.len() < 6 {
        return Err(AppError::BadRequest(
            "Password must be at least 6 characters".into(),
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

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Account created successfully",
            "email": email,
            "plan": plan_slug,
        })),
    ))
}
