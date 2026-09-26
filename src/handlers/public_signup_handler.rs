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
use sqlx::Row;
use uuid::Uuid;

use crate::auth::models::Claims;

/// Does this `?ref=` code exist? (kanban t_8101edb5, arm (b)).
///
/// The chosen namespace is `tenants.affiliate_code` — the code `kinetic_handler` renders into the
/// card badge `?ref=` URL and the code `affiliate_referral_handler` publishes as the share link.
/// The two legacy namespaces (`affiliates.id`, `affiliate_links.tracking_code`) are resolved too,
/// exactly as the reader does, so this verdict can never disagree with the reader's `resolved`.
/// Always returns one row (bare SELECT), NULLs when nothing matches.
const RESOLVE_SQL: &str = "\
SELECT (SELECT t.id   FROM tenants t WHERE t.affiliate_code = $1 ORDER BY t.created_at LIMIT 1) AS tenant_id,
       (SELECT t.name FROM tenants t WHERE t.affiliate_code = $1 ORDER BY t.created_at LIMIT 1) AS tenant_name,
       (SELECT af.name FROM affiliates af
         WHERE af.id = COALESCE((SELECT al.affiliate_id FROM affiliate_links al
                                  WHERE al.tracking_code = $1 LIMIT 1), $1)
         LIMIT 1) AS affiliate_name";

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
    // (b) VALIDATE THE WRITE — decided arm: STILL-RECORD-AND-FLAG (kanban t_8101edb5).
    //
    // A `?ref=` that resolves to nothing must NOT refuse the signup: the code is copied by hand,
    // shared in DMs and printed on cards, so a typo or a dead link is an ordinary event, and
    // refusing the signup to punish a bad link throws away the customer instead of the code.
    // Sanitising is enforced (trim, empty -> none, cap 100 = the width of `tenants.affiliate_code`);
    // the row is then recorded exactly as it always was, and the verdict is FLAGGED in three places
    // that outlive this request: the `referral.resolved` field below, the log line, and the
    // recomputed `resolved:false` on `GET /api/v1/affiliate-referrals`.
    let affiliate_code = payload["affiliate_code"]
        .as_str()
        .map(|s| s.trim().chars().take(100).collect::<String>())
        .filter(|s| !s.is_empty());

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

    // Handle affiliate referral code. The code is resolved against the namespace the rest of the
    // app publishes; an unresolved code is recorded anyway and FLAGGED (arm (b), t_8101edb5) — see
    // `RESOLVE_SQL` and the comment where `affiliate_code` is parsed.
    let mut referral: Option<Value> = None;
    if let Some(ref_code) = affiliate_code {
        let row = sqlx::query(RESOLVE_SQL)
            .bind(&ref_code)
            .fetch_one(&state.pool)
            .await?;
        let owner_id: Option<Uuid> = row.try_get("tenant_id")?;
        let owner_name: Option<String> = row.try_get("tenant_name")?;
        let affiliate_name: Option<String> = row.try_get("affiliate_name")?;
        let resolved = owner_id.is_some() || affiliate_name.is_some();

        if !resolved {
            tracing::warn!(
                affiliate_code = %ref_code,
                signup_email = %email,
                "Unresolved affiliate code: signup recorded and flagged, not refused"
            );
        }

        let _ = sqlx::query(
            r#"INSERT INTO referral_tracking (id, referrer_code, referred_email, referred_tenant_id, created_at)
               VALUES ($1, $2, $3, $4, NOW())"#,
        )
        .bind(Uuid::new_v4())
        .bind(&ref_code)
        .bind(&email)
        .bind(tenant_id)
        .execute(&state.pool)
        .await;

        referral = Some(json!({
            "code": ref_code,
            "resolved": resolved,
            "referrer_tenant": owner_id.map(|id| json!({
                "tenant_id": id.to_string(),
                "name": owner_name.clone().unwrap_or_default(),
            })),
            "affiliate_name": affiliate_name,
        }));
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
            "referral": referral,
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
