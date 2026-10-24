use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use axum::{
    extract::{Json, Path, Query, State},
    response::IntoResponse,
};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use rand::rngs::OsRng;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::auth::models::Claims;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Admin-only sync endpoint called by CoreSwift to create portfolio company with user account
pub async fn portfolio_sync(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<impl IntoResponse> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let sync_id = req
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4);
    let name = req
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Company")
        .to_string();
    let email = req
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let description = req
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Normalise + validate BEFORE the first SELECT (kanban t_38017305). This is a writer of
    // `users.email` (admin / cross-app create), so it must refuse a non-address exactly like the
    // public signup paths do — and it must check for a duplicate on the same normalised value it
    // stores. The old `email.is_empty()` guard let `bad` through.
    let email =
        crate::security::email_addr::normalize(&email).map_err(AppError::UnprocessableEntity)?;

    // Idempotency on the sync `id` (kanban t_149639f7).
    //
    // This door PROVISIONS ACCOUNTS: it mints a `tenants` row (with its own plan subscription), an
    // argon2-hashed `users` row and a generated credential BEFORE the `portfolio_companies` upsert
    // further down. The upsert is keyed on the sync `id`; the mint was not — so a REPEAT push of a
    // company that already had an account ran the mint again. Because the tenant slug is derived
    // deterministically from the company name, that second mint died on `tenants_slug_key` and the
    // route answered `500 Database error` with NOTHING refreshed (measured live pre-fix: push2 of one
    // id answered 500, the company row still carried the push1 email/description), and a repeat that
    // reached the email check at all answered 409 — so a portfolio sync could never simply be re-run.
    //
    // The account is keyed on the COMPANY, so look the company up FIRST: an id we already hold is a
    // refresh, never a re-provision. Only the descriptive columns move (name, email, description) —
    // NOT `tenant_id` (the scoping key, and the parent of the company's own children, which carry
    // their own) and NOT `slug` (bound at mint to its OWN tenant's slug; re-deriving it would
    // manufacture drift). This is the contract the fleet settled on for the twin door
    // (MissedCall Respondr, t_ff373bd1) and in t_e63b1450's verdict on the upsert's DO UPDATE arm,
    // which this branch mirrors by construction.
    let owner_tenant_id: Option<Uuid> =
        sqlx::query_scalar("SELECT tenant_id FROM portfolio_companies WHERE id = $1")
            .bind(sync_id)
            .fetch_optional(&state.pool)
            .await?;

    if let Some(account_id) = owner_tenant_id {
        sqlx::query(
            "UPDATE portfolio_companies SET name = $2, email = $3, description = $4, updated_at = NOW() WHERE id = $1",
        )
        .bind(sync_id)
        .bind(&name)
        .bind(&email)
        .bind(&description)
        .execute(&state.pool)
        .await?;

        // 200, the account that already owns the company, and NO `user_id`/`password` — nothing was
        // minted this time, so there is no credential to share and no credential mail is sent. Same
        // answer shape the twin door gives (`already_exists`).
        return Ok(Json(json!({
            "status": "already_exists",
            "id": sync_id.to_string(),
            "name": name,
            "email": email,
            "account_id": account_id.to_string(),
            "note": "This company already has an account; its contact details were refreshed and no new credentials were created."
        })));
    }

    // Check email uniqueness
    let existing =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0_i64);

    if existing > 0 {
        return Err(AppError::Conflict(format!(
            "A user with email {} already exists",
            email
        )));
    }

    // Create tenant
    let tenant_id = Uuid::new_v4();
    let tenant_slug = crate::security::display_name::slugify(&name);

    sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind(&name)
        .bind(&tenant_slug)
        .execute(&state.pool)
        .await?;

    // Portfolio tenants are sister companies David owns, so they always get THIS app's highest plan
    // (David, 2026-10-04: "all portfolio tenants are sister companies that I own. So they always get
    // the highest level of each software").
    //
    // Resolve it the way the console documents — the first ACTIVE plan in its own order — instead of
    // binding a plan NAME. The previous code asked for name = 'Enterprise', which no live install has,
    // so the lookup returned None and the synced tenant silently ended up on no plan at all. The
    // INSERT's result was discarded too, which is why nothing ever reported it.
    let plan_id: Option<Uuid> = sqlx::query_scalar(
        // Its price column is `price` (no is_active, no price_monthly) - checked on the live schema.
        "SELECT id FROM plans ORDER BY price DESC NULLS LAST, slug ASC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;

    let pid = plan_id.ok_or_else(|| {
        AppError::BadRequest("no active plan exists to place a portfolio tenant on".into())
    })?;

    sqlx::query(
        "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date) VALUES ($1, $2, $3, 'active', NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(pid)
    .execute(&state.pool)
    .await?;

    // Create user
    let user_id = Uuid::new_v4();
    let generated_password = Uuid::new_v4()
        .to_string()
        .replace("-", "")
        .chars()
        .take(12)
        .collect::<String>();
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
    // ── David: *"if the admin creates a user the same thing happens."* ───────────────────────────
    // This function already generated `generated_password` from the OS RNG and hashed it — and then
    // sent it NOWHERE. The account existed with a password nobody knew, so the user could never sign
    // in and the admin had nothing to hand over. The generated value is emailed here, loudly on
    // failure, for the same reason as the signup path.
    match crate::email::send_credentials_email(
        &state.pool,
        tenant_id,
        &email,
        &name,
        &generated_password,
    )
    .await
    {
        Ok(()) => tracing::info!(email = %email, "admin: login credentials emailed"),
        Err(e) => tracing::error!(
            email = %email,
            error = %e,
            "admin: user created but the CREDENTIAL EMAIL FAILED — they have no password."
        ),
    }

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
        (
            "http://localhost:8087/api/v1/internal/portfolio-companies",
            "ADASwift",
        ),
        (
            "http://localhost:8088/api/v1/internal/portfolio-companies",
            "MissedCall",
        ),
        (
            "http://localhost:8085/api/v1/internal/portfolio-companies",
            "WorkflowSwift",
        ),
        (
            "http://localhost:8084/api/portfolio/internal",
            "CoreSwift CRM",
        ),
        (
            // IncentiveSwift serves this handler at /api/v1/internal/portfolio-companies
            // (src/main.rs); the /api/internal/... path this list used to carry is a CoreSwift
            // namespace and answered 404, so the IncentiveSwift leg of every tenant sync never
            // landed a row (kanban t_8b887f91 — measured 404 before the fix).
            "http://localhost:8083/api/v1/internal/portfolio-companies",
            "IncentiveSwift",
        ),
    ];
    // One client for all five legs, with a per-leg timeout: each leg used to build its own
    // `reqwest::Client::new()` with no timeout, so one hung receiver stalled this route forever
    // (kanban t_fe17480d). 5s matches CoreSwift-CRM's broadcast_portfolio_sync.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|e| {
            tracing::error!("Portfolio sync: could not build the HTTP client: {}", e);
            reqwest::Client::new()
        });
    // Per-leg result, carried in the response (kanban t_fe17480d). The route used to answer
    // {"status":"synced"} with HTTP 200 no matter how many legs failed, and a failed leg was only a
    // warn line naming the app and status — which is how two dead receivers went unnoticed for
    // months. A caller can now tell full success from partial success without reading logs.
    let mut legs: Vec<Value> = Vec::with_capacity(apps.len());
    let mut failed: Vec<String> = Vec::new();
    for (url, app_name) in apps {
        match client
            .post(url)
            .header("x-internal-key", key)
            .header("Content-Type", "application/json")
            .json(&sync_body)
            .send()
            .await
        {
            Ok(resp) => {
                let http_status = resp.status();
                if http_status.is_success() {
                    legs.push(json!({"app": app_name, "ok": true, "status": http_status.as_u16()}));
                } else {
                    // Cap what a receiver can put in the log line / the response body.
                    let detail: String = resp
                        .text()
                        .await
                        .unwrap_or_default()
                        .trim()
                        .chars()
                        .take(200)
                        .collect();
                    tracing::error!(
                        "Portfolio sync to {} REJECTED: HTTP {} — {}",
                        app_name,
                        http_status,
                        detail
                    );
                    failed.push(app_name.to_string());
                    legs.push(json!({
                        "app": app_name,
                        "ok": false,
                        "status": http_status.as_u16(),
                        "error": format!("HTTP {}: {}", http_status, detail)
                    }));
                }
            }
            Err(e) => {
                // Transport failure (refused / timeout / bad body) — distinct from a rejection.
                let detail: String = e.to_string().chars().take(200).collect();
                tracing::error!(
                    "Portfolio sync to {} FAILED (transport): {}",
                    app_name,
                    detail
                );
                failed.push(app_name.to_string());
                legs.push(json!({
                    "app": app_name,
                    "ok": false,
                    "error": format!("transport: {}", detail)
                }));
            }
        }
    }
    let legs_failed = failed.len();
    let legs_synced = legs.len() - legs_failed;
    // Same class of success answer as before when every leg lands ("synced"); a partial fan-out now
    // says so in `status` and lists the legs that failed. HTTP stays 200 either way: the tenant, its
    // plan subscription, its user and its portfolio row are already committed above, so the fan-out
    // outcome is a body-level fact, not a reason to fail the caller's request.
    let sync_status = if legs_failed == 0 {
        "synced"
    } else {
        "partial"
    };

    Ok(Json(json!({
        "status": sync_status,
        "legs_synced": legs_synced,
        "legs_failed": legs_failed,
        "failed": failed,
        "legs": legs,
        "message": if legs_failed == 0 {
            format!("Synced to all {} receivers", legs.len())
        } else {
            format!(
                "Synced to {}/{} receivers — failed: {}",
                legs_synced,
                legs.len(),
                failed.join(", ")
            )
        },
        "id": sync_id.to_string(),
        "name": name,
        "email": email,
        "tenant_id": tenant_id.to_string(),
        "user_id": user_id.to_string(),
        "password": generated_password,
        "note": "Share credentials with the company. They can login directly."
    })))
}

/// Admin impersonation — generates a JWT that impersonates the target tenant account.
/// Admin's own account_id is stored in the impersonating claim so the UI can show a banner.
pub async fn impersonate(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<impl IntoResponse> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let account_id = body
        .get("account_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("account_id is required".into()))?;

    let tid = Uuid::parse_str(account_id)
        .map_err(|_| AppError::BadRequest("Invalid account_id format".into()))?;

    // Look up the target tenant
    let target = sqlx::query_as::<_, crate::auth::models::Tenant>(
        "SELECT id, name, slug, logo, colors, settings, created_at, updated_at FROM tenants WHERE id = $1"
    )
    .bind(tid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::BadRequest("Tenant not found".into()))?;

    // tenants.status enforcement, part 4 of 4 (kanban t_af890bbf): a retired workspace cannot be
    // impersonated. The session this would mint carries the tenant's own role, so the central gate
    // would refuse every call it made — answering 200 here would hand the admin a token that is
    // dead on arrival and look like a bug in the console. Admin-only route, and the target is a
    // different tenant than the caller's, so this is the one place the gate cannot cover.
    if crate::handlers::tenant_handler::tenant_status_for(&state.pool, tid)
        .await?
        .as_deref()
        != Some("active")
    {
        return Err(AppError::Forbidden(
            "This workspace is inactive and cannot be impersonated.".into(),
        ));
    }

    // Get a user for this tenant to use as the impersonated identity
    let target_user = sqlx::query_as::<_, crate::auth::models::User>(
        "SELECT id, tenant_id, email, password_hash, name, role, is_active, created_at, updated_at FROM users WHERE tenant_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(tid)
    .fetch_optional(&state.pool)
    .await?;

    let now = Utc::now().timestamp() as usize;
    let exp = now + 3600; // 1 hour

    let imp_claims = Claims {
        sub: target_user
            .as_ref()
            .map(|u| u.id.to_string())
            .unwrap_or_else(|| tid.to_string()),
        tenant_id: tid.to_string(),
        email: target_user
            .as_ref()
            .map(|u| u.email.clone())
            .unwrap_or_else(|| format!("tenant-{}@funnelswift.internal", account_id)),
        role: target_user
            .as_ref()
            .map(|u| u.role.clone())
            .unwrap_or_else(|| "member".to_string()),
        exp,
        iat: now,
        aud: Some("funnelswift-api".to_string()),
        iss: Some("funnelswift".to_string()),
        impersonating: Some(auth.user_id.clone()),
    };

    let token = encode(
        &Header::default(),
        &imp_claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {}", e)))?;

    Ok(Json(json!({
        "token": token,
        "impersonating": {
            "id": tid.to_string(),
            "name": target.name,
            "email": target_user.as_ref().map(|u| u.email.as_str()).unwrap_or("")
        },
        "expires_in": 3600,
        "token_type": "Bearer"
    })))
}

/// Stop impersonation — called BY THE IMPERSONATION SESSION ITSELF, not by the admin.
///
/// The session that needs this route is the one carrying the impersonated account's token, and
/// [`impersonate`] mints that token with the TARGET USER'S role (`role` above), so `is_admin` is
/// the tenant's role there and gating on it made this route unreachable from its only caller: the
/// console's Stop answered `403 Admin access required` (measured live on admin.funnelswift.net).
/// The honest test for "this is an impersonation session" is the `impersonating` claim the
/// impersonate handler sets. A plain admin token may call it too — for an admin it is a
/// no-op acknowledgement, which is what the route always was (the tokens are stateless, so
/// ending the session is the caller dropping the impersonation token).
pub async fn stop_impersonation(auth: AuthUser) -> AppResult<impl IntoResponse> {
    if !auth.is_admin && auth.impersonating.is_none() {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    Ok(Json(json!({
        "status": "impersonation_stopped",
        "note": "Drop impersonation token. Restore your original admin token."
    })))
}

/// Platform-wide counters for the Super Admin dashboard — `GET /api/v1/admin/stats`.
///
/// The served admin console (`www-admin/index.html`, `renderSuperAdmin()`) has always rendered four
/// counters read from this path, but no such route existed anywhere in the backend: the call
/// answered 404 and the console's `try/catch` left `stats = {}`, so every card on the Super Admin
/// dashboard read 0. The four JSON keys are the console's own. `plans` has no active/inactive
/// column, so `total_plans` is the plan-table count (the console labels it "Active Plans").
/// Admin-gated: these are platform-wide numbers, not tenant-scoped ones.
///
/// HARNESS TRAFFIC IS NOT A CUSTOMER (measured live 2026-10-21). The Tenants list this dashboard
/// sits above already hides a harness-tagged workspace (`tenants.probe_harness`, stamped at signup
/// by `crate::probe_harness::for_signup`; see `tenant_handler::list_tenants`). These counters did
/// not, so the headline said "N tenants" while the list underneath showed N-1 and the operator had
/// no way to tell which number was true. Both now apply the SAME predicate: a tagged workspace is
/// excluded unless the caller passes `?include_probe=1`, and a workspace minted before the column
/// existed (`probe_harness` NULL) is a customer as far as this count is concerned. `^CNRY` rows stay
/// visible, exactly as the list keeps them, so the fleet's own canaries remain observable.
pub async fn admin_stats(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let include_probe = params
        .get("include_probe")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // One predicate, spelled twice exactly as the list spells it: a workspace is COUNTED only
    // when it would be LISTED. `$1::bool OR COALESCE(t.probe_harness,'') = '' OR t.probe_harness ~* '^CNRY'`
    let total_tenants: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenants t \
         WHERE $1::bool OR COALESCE(t.probe_harness, '') = '' OR t.probe_harness ~* '^CNRY'",
    )
    .bind(include_probe)
    .fetch_one(&state.pool)
    .await?;
    let total_users: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users u JOIN tenants t ON t.id = u.tenant_id \
         WHERE $1::bool OR COALESCE(t.probe_harness, '') = '' OR t.probe_harness ~* '^CNRY'",
    )
    .bind(include_probe)
    .fetch_one(&state.pool)
    .await?;
    let total_affiliates: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM affiliates")
        .fetch_one(&state.pool)
        .await?;
    let total_plans: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({
        "total_tenants": total_tenants,
        "total_users": total_users,
        "total_affiliates": total_affiliates,
        "total_plans": total_plans,
    })))
}

/// Set, replace or clear a tenant's single feature override (admin only).
/// POST /api/v1/admin/tenants/:id/feature-override
/// Body: { "feature_key": "<registry key>", "limit_value": <int|null> }
///
///   * `limit_value: null` -> CLEAR the override; the tenant's plan decides again.
///   * numeric key -> the effective cap: -1 unlimited, 0 disabled, >0 capped (same convention as a
///     `feature_limits` row).
///   * boolean key -> 1 grants, 0 refuses.
///
/// The key is validated against `feature_registry::spec`, so a retired or unknown name is refused
/// with 400 exactly like `PUT /api/v1/admin/plans/entitlement` — a `feature_override` row that
/// names a key no gate reads would be the very knob this endpoint used to be. The value is
/// validated against the key's kind, so every stored row is something the resolvers can read.
///
/// The override is consulted by every gate BEFORE the plan's own row/column (kanban t_0a139e57,
/// precedence in `features::pick_limit`), so this endpoint moves a trade from the moment it lands.
pub async fn set_tenant_feature_override(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let feature_key = req
        .get("feature_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("feature_key is required".into()))?;

    let spec = crate::feature_registry::spec(feature_key).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown feature '{feature_key}' — it is not in the plan feature registry"
        ))
    })?;

    let limit_value = match req.get("limit_value") {
        None => {
            return Err(AppError::BadRequest(
                "limit_value is required (an integer, or null to clear the override)".into(),
            ))
        }
        Some(Value::Null) => None,
        Some(v) => Some(
            v.as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "limit_value must be an integer, or null to clear the override".into(),
                    )
                })?,
        ),
    };

    if let Some(v) = limit_value {
        if !crate::feature_registry::override_value_ok(spec.kind, v) {
            return Err(AppError::BadRequest(format!(
                "'{feature_key}' is a boolean feature — limit_value must be 1 (grant) or 0 \
                 (refuse); send null to clear the override"
            )));
        }
    }

    let status = match limit_value {
        // Clearing is a DELETE, so the slot never holds a number the resolver would have to guess
        // at. `id` has no column default on tenant_settings, so it is generated here — omitting it
        // is what made this endpoint answer 500 for every caller before kanban t_0a139e57.
        None => {
            sqlx::query(
                "DELETE FROM tenant_settings WHERE tenant_id = $1 AND key = 'feature_override'",
            )
            .bind(tenant_id)
            .execute(&state.pool)
            .await?;
            "cleared"
        }
        Some(v) => {
            sqlx::query(
                r#"INSERT INTO tenant_settings (id, tenant_id, key, value)
                   VALUES (gen_random_uuid(), $1, 'feature_override', $2::jsonb)
                   ON CONFLICT (tenant_id, key) DO UPDATE SET value = $2::jsonb, updated_at = NOW()"#,
            )
            .bind(tenant_id)
            .bind(json!({
                "feature_key": feature_key,
                "limit_value": v
            }))
            .execute(&state.pool)
            .await?;
            "set"
        }
    };

    Ok(Json(json!({
        "message": if status == "set" { "Feature override set" } else { "Feature override cleared" },
        "status": status,
        "tenant_id": tenant_id.to_string(),
        "feature_key": feature_key,
        "limit_value": limit_value,
        "kind": match spec.kind {
            crate::feature_registry::Kind::Boolean => "boolean",
            crate::feature_registry::Kind::Limit => "limit",
        },
        "precedence": "tenant override > plan feature_limits row > plan column",
    })))
}

/// Get tenant-level feature overrides (admin only).
/// GET /api/v1/admin/tenants/:id/feature-overrides
pub async fn get_tenant_feature_overrides(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let overrides = sqlx::query_as::<_, (String, Value)>(
        "SELECT 'feature_override' as key, value FROM tenant_settings WHERE tenant_id = $1 AND key = 'feature_override'"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({
        "tenant_id": tenant_id.to_string(),
        "overrides": overrides.iter().map(|(_, v)| v).collect::<Vec<_>>()
    })))
}

/// List users for a specific tenant (admin only)
pub async fn list_tenant_users(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tid = tenant_id;

    let users = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            Option<String>,
            String,
            bool,
            String,
        ),
    >(
        r#"SELECT id, email, first_name, last_name, role, is_active, name
           FROM users WHERE tenant_id = $1
           ORDER BY created_at DESC"#,
    )
    .bind(tid)
    .fetch_all(&state.pool)
    .await?;

    let result: Vec<Value> = users
        .into_iter()
        .map(
            |(id, email, first_name, last_name, role, is_active, name)| {
                json!({
                    "id": id.to_string(),
                    "email": email,
                    "name": name,
                    "first_name": first_name,
                    "last_name": last_name,
                    "role": role,
                    "is_active": is_active,
                })
            },
        )
        .collect();

    Ok(Json(json!(result)))
}

/// The roles a teammate may be minted with, or moved to.
///
/// This is the vocabulary the live `users` table actually holds (measured 2026-10-18:
/// user / admin / company_admin / business_user); `member` is not one of them and no gate in this
/// app recognises it. One list, shared by the mint and the role change, so the two can never drift.
pub const TEAMMATE_ROLES: [&str; 4] = ["user", "admin", "company_admin", "business_user"];

/// Parse a requested teammate role: absent/blank keeps the member default, anything outside
/// [`TEAMMATE_ROLES`] is a 400 that names the field instead of a row no gate would recognise.
fn teammate_role(raw: Option<&str>) -> Result<String, AppError> {
    let role = raw
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "user".to_string());
    if !TEAMMATE_ROLES.contains(&role.as_str()) {
        return Err(AppError::BadRequest(format!(
            "role must be one of: {}",
            TEAMMATE_ROLES.join(", ")
        )));
    }
    Ok(role)
}

/// Mint a user INSIDE an existing workspace (Super Admin only).
///
/// `POST /api/v1/admin/users` with `{tenant_id, name, email}`. Until now the console could only
/// LIST a workspace's people (`POST /admin/tenants/:id/users`) — there was no route that added one,
/// so the "Users" count on the Tenants table was a dead number and an operator had no way to add a
/// teammate to a customer's workspace.
///
/// The mint follows the same contract as the first-user mint: generate a password, hash it, and mail
/// it. The failure mode that contract left open is the one this route closes — when the credentials
/// mail cannot go out (no provider configured, provider refused) or is deliberately withheld (a
/// reserved/harness address is never attempted), the account exists with a password nobody knows.
/// So the answer states the outcome: `credentials_emailed` says the provider accepted the message,
/// and when it did not, `handover_password` carries the generated value for THIS response only, so
/// the operator can pass it to the customer.
pub async fn create_tenant_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id = req
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("tenant_id must be a uuid".into()))?;
    let email = req
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest(
            "a valid email address is required".into(),
        ));
    }
    let given = req
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let name = if given.is_empty() {
        email.split('@').next().unwrap_or("Teammate").to_string()
    } else {
        given
    };

    // The workspace must exist: a mint against a stale id would leave an orphan user row.
    let tenant_name: Option<String> = sqlx::query_scalar("SELECT name FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?;
    let tenant_name = tenant_name.ok_or_else(|| AppError::NotFound("No such workspace".into()))?;

    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE lower(email) = $1")
        .bind(&email)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
    if taken > 0 {
        return Err(AppError::Conflict(format!(
            "{email} already has an account"
        )));
    }

    let generated_password = Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(12)
        .collect::<String>();
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(generated_password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();
    let user_id = Uuid::new_v4();
    // The ROLE the teammate lands on. It used to be the literal 'user' for every mint, so an
    // operator adding a co-admin or a company admin had no way to say so and the panel could only
    // ever add members (2026-10-18).
    let role = teammate_role(req.get("role").and_then(|v| v.as_str()))?;
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, true, NOW(), NOW())",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .bind(&role)
    .execute(&state.pool)
    .await?;

    // `harness_domain` is the fleet's ONE suppression test: a reserved address is never sent to, so
    // nothing leaves and nothing can bounce. It is not a failure, but it is also not a delivery —
    // the operator still has to hand the password over.
    let withheld = crate::security::probe_addr::harness_domain(&email).is_some();
    let mut emailed = false;
    let mut detail = String::new();
    if !withheld {
        match crate::email::send_credentials_email(
            &state.pool,
            tenant_id,
            &email,
            &name,
            &generated_password,
        )
        .await
        {
            Ok(()) => emailed = true,
            Err(e) => detail = e,
        }
    }

    Ok(Json(json!({
        "id": user_id.to_string(),
        "tenant_id": tenant_id.to_string(),
        "tenant_name": tenant_name,
        "email": email,
        "name": name,
        "role": role,
        "credentials_emailed": emailed,
        "withheld": withheld,
        "detail": detail,
        "handover_password": if emailed { Value::Null } else { json!(generated_password) },
    })))
}

/// Move an existing teammate to another role (admin only).
///
/// `POST /api/v1/admin/users/:id/role` with `{role}`. The console could add a teammate but never
/// change the role it had put them on, so the only way to correct a wrong role was direct SQL.
/// Two invariants live here rather than in the operator's memory: nobody can change their OWN role
/// (a super admin who demoted themselves would lose the console mid-session), and the LAST active
/// admin of a workspace cannot be demoted (that leaves the workspace with nobody who can administer
/// it). Both answer 400 with the reason, so the panel can print the sentence.
pub async fn set_tenant_user_role(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let role = teammate_role(req.get("role").and_then(|v| v.as_str()))?;

    let row: Option<(Uuid, Uuid, String, String)> =
        sqlx::query_as("SELECT id, tenant_id, email, role FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?;
    let (uid, tenant_id, email, current) =
        row.ok_or_else(|| AppError::NotFound("No such teammate".into()))?;

    if current == role {
        return Ok(Json(json!({
            "id": uid.to_string(), "email": email, "role": role, "changed": false
        })));
    }
    if uid.to_string() == auth.user_id {
        return Err(AppError::BadRequest(
            "you cannot change your own role - ask another admin".into(),
        ));
    }
    if current == "admin" {
        let admins: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE tenant_id = $1 AND role = 'admin' AND is_active",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        if admins <= 1 {
            return Err(AppError::BadRequest(
                "that is the workspace's only admin - promote someone else first".into(),
            ));
        }
    }
    sqlx::query("UPDATE users SET role = $1, updated_at = NOW() WHERE id = $2")
        .bind(&role)
        .bind(uid)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({
        "id": uid.to_string(), "email": email, "role": role, "changed": true
    })))
}

/// Addresses this install must never remove through the panel: the platform operator and the
/// portfolio workspaces' owners. The removal route is destructive, so this is a hard deny-list
/// rather than a role check — it holds even when the caller IS an admin of that workspace.
const PROTECTED_TEAMMATE_EMAILS: &[&str] = &[
    "swiftsoftware143@yahoo.com",
    "david@funnelswift.net",
    "swiftdavid@yahoo.com",
    "swiftimpactsolutions@gmail.com",
    "zaarhub@gmail.com",
    "giraudycapital@yahoo.com",
    "mcr_admin@swiftsoftware.com",
];

/// **Remove a teammate from a workspace** — `DELETE /api/v1/admin/users/:id` (Super Admin only).
///
/// The panel could ADD a teammate and move one between roles, but never take one out: a typo, a
/// duplicate invite or someone who has left kept a working login for ever, and the only remedy was
/// direct SQL. Same gap the avatar DELETE closed on the profile.
///
/// TWO modes, because the row alone is not the whole story for a teammate who has done work:
///  * **no history** — the `users` row is DELETEd and `user_avatars`, `password_resets`,
///    `email_changes` and `linkedin_auths` cascade with it (`removed: true`).
///  * **has history** — `leads.created_by` / `leads.assigned_to` and
///    `affiliate_payouts.affiliate_user_id` are `NO ACTION`, so a DELETE would answer 23503 and
///    forcing it would strip those rows of their owner. The row is DEACTIVATED instead
///    (`deactivated: true`): the login query requires `is_active = true` and every authenticated
///    request re-reads it (`global_auth`), so the teammate's live session dies at once while their
///    leads and payouts keep their owner.
///
/// Refused 400 with a sentence the pane shows verbatim: yourself, the workspace's only active
/// admin, and [`PROTECTED_TEAMMATE_EMAILS`].
pub async fn delete_tenant_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let row: Option<(Uuid, Uuid, String, String, bool)> =
        sqlx::query_as("SELECT id, tenant_id, email, role, is_active FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?;
    let (uid, tenant_id, email, role, is_active) =
        row.ok_or_else(|| AppError::NotFound("No such teammate".into()))?;

    if uid.to_string() == auth.user_id {
        return Err(AppError::BadRequest(
            "you cannot remove yourself from the workspace".into(),
        ));
    }
    let lower = email.to_lowercase();
    if PROTECTED_TEAMMATE_EMAILS.contains(&lower.as_str()) {
        return Err(AppError::BadRequest(
            "that address is protected and cannot be removed from the panel".into(),
        ));
    }
    if role == "admin" && is_active {
        let admins: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE tenant_id = $1 AND role = 'admin' AND is_active",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        if admins <= 1 {
            return Err(AppError::BadRequest(
                "that is the workspace's only admin - promote someone else first".into(),
            ));
        }
    }

    // History decides the mode: these two FKs are NO ACTION, so a hard delete is only honest for a
    // teammate who never owned anything in this workspace.
    let has_history: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM leads WHERE created_by = $1 OR assigned_to = $1)
             OR EXISTS(SELECT 1 FROM affiliate_payouts WHERE affiliate_user_id = $1)",
    )
    .bind(uid)
    .fetch_one(&state.pool)
    .await?;

    if has_history {
        sqlx::query("UPDATE users SET is_active = false, updated_at = NOW() WHERE id = $1")
            .bind(uid)
            .execute(&state.pool)
            .await?;
        return Ok(Json(json!({
            "id": uid.to_string(),
            "email": email,
            "removed": false,
            "deactivated": true,
            "reason": "this teammate owns leads or payouts, so their record is kept - the login is switched off"
        })));
    }

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(uid)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({
        "id": uid.to_string(),
        "email": email,
        "removed": true,
        "deactivated": false
    })))
}

/// List ALL kinetic cards across all tenants (admin only)
pub async fn admin_list_all_cards(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    // Three things this statement got wrong, all read-side:
    //  * `kc.subdomain` does not exist on kinetic_cards (42703 — the endpoint 500'd for
    //    every caller). The tenant's subdomain lives in tenant_settings under key
    //    'subdomain', the same source kinetic_handler::get_subdomain reads;
    //    UNIQUE(tenant_id, key) means the extra LEFT JOIN cannot multiply rows.
    //  * kinetic_cards.theme is NULLABLE and NULL for 18 of 27 live cards -> Option,
    //    rendered as "" exactly like kinetic_handler's own card list (line 87).
    //  * kinetic_cards.template_type is NULLABLE with DEFAULT 'default' -> COALESCE.
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            bool,
            i64,
            Option<String>,
        ),
    >(
        // The "Tenant" column names the WORKSPACE a card belongs to, so the name comes from
        // `tenants` — the owning row. This query used to join `portfolio_companies` (the tenant's
        // own list of client companies, a different thing entirely): it answered 'Unknown' for
        // every card on this install (measured 2026-10-19: 7 cards, 0 portfolio_companies rows)
        // and would have multiplied a card once per portfolio company for a tenant that had any.
        //  * the console's "All Cards" table has always rendered a Views column and a UTM badge,
        //    and this payload carried NEITHER, so every row showed 0 views and "None" whatever the
        //    card had actually done. The two correlated reads below are the same sources the
        //    per-card analytics screens use: `kinetic_card_daily_stats.views` (summed per card) and
        //    the newest non-null `utm_source` in `kinetic_card_events`. Both are scalar subqueries,
        //    so the row count cannot multiply.
        r#"SELECT kc.id, kc.tenant_id, COALESCE(t.name, 'Unknown') as tenant_name,
               kc.title, kc.slug, COALESCE(kc.template_type, 'default') as template_type, kc.theme,
               COALESCE(ts.value #>> '{}', '') as subdomain, kc.avatar_url, kc.is_active,
               COALESCE((SELECT SUM(s.views) FROM kinetic_card_daily_stats s
                          WHERE s.card_id = kc.id), 0)::bigint as view_count,
               (SELECT e.utm_source FROM kinetic_card_events e
                 WHERE e.card_id = kc.id AND e.utm_source IS NOT NULL AND e.utm_source <> ''
                 ORDER BY e.created_at DESC LIMIT 1) as utm_source
        FROM kinetic_cards kc
        LEFT JOIN tenants t ON t.id = kc.tenant_id
        LEFT JOIN tenant_settings ts ON ts.tenant_id = kc.tenant_id AND ts.key = 'subdomain'
        ORDER BY kc.updated_at DESC
        LIMIT 200"#,
    )
    .fetch_all(&state.pool)
    .await?;

    let cards: Vec<Value> = rows
        .iter()
        .map(
            |(
                id,
                tid,
                tenant_name,
                title,
                slug,
                template_type,
                theme,
                subdomain,
                avatar_url,
                is_active,
                view_count,
                utm_source,
            )| {
                json!({
                    "id": id.to_string(),
                    "tenant_id": tid.to_string(),
                    "tenant_name": tenant_name,
                    "title": title,
                    "slug": slug,
                    "template_type": template_type,
                    "theme": theme.as_deref().unwrap_or(""),
                    "subdomain": subdomain,
                    "avatar_url": avatar_url,
                    "is_active": is_active,
                    "view_count": view_count,
                    "utm_source": utm_source,
                })
            },
        )
        .collect();

    Ok(Json(json!({"cards": cards, "total": cards.len()})))
}

/// Query string for [`list_provisioning_log`]. `limit` is clamped server-side.
#[derive(serde::Deserialize)]
pub struct ProvisionLogQuery {
    pub limit: Option<i64>,
}

/// List recent free-account provisioning attempts for the caller's tenant (admin only).
///
/// `provisioning_log` has recorded every tag-driven provisioning attempt since its migration
/// (one row per attempt, written `pending` before the sibling call and updated with the real
/// outcome), and the admin guide tells the operator "every attempt is visible in the provisioning
/// log". Until this route the table had a writer and NO door onto its rows for the operator: the
/// only reader was `app_provision::history_for_lead`, called from the probe harness, and the console
/// had no view — so the guide promised a screen that did not exist. This is that screen's reader.
///
/// Tenant-scoped on purpose: an admin sees only their own leads' attempts. Newest first.
pub async fn list_provisioning_log(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<ProvisionLogQuery>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        String,
        Option<String>,
        String,
        Option<i32>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, lead_id, source_app, plan_slug, status, http_status, error_message, created_at \
         FROM provisioning_log WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let attempts: Vec<Value> = rows
        .iter()
        .map(
            |(
                id,
                lead_id,
                source_app,
                plan_slug,
                status,
                http_status,
                error_message,
                created_at,
            )| {
                json!({
                    "id": id.to_string(),
                    "lead_id": lead_id.map(|l| l.to_string()),
                    "source_app": source_app,
                    "plan_slug": plan_slug,
                    "status": status,
                    "http_status": http_status,
                    "error_message": error_message,
                    "created_at": created_at.to_rfc3339(),
                })
            },
        )
        .collect();

    Ok(Json(
        json!({ "count": attempts.len(), "attempts": attempts }),
    ))
}

/// Query string for [`list_checkout_requests`]. `limit` is clamped server-side.
#[derive(serde::Deserialize)]
pub struct CheckoutRequestQuery {
    pub limit: Option<i64>,
}

/// List purchase attempts that never turned into money, across every workspace (admin only).
///
/// `POST /api/v1/checkout/create` writes a `checkout_sessions` row for every attempt and, when the
/// installation has no payment provider, records it as `provider_missing` and tells the customer
/// *"your request has been recorded for the platform team, who can activate this plan for your
/// workspace by hand"*. Until this route that promise had no screen behind it: the row was written
/// and nothing in either console read it, so a customer asking to buy produced a record the
/// operator could never see (measured 2026-10-23: one real tenant's refused attempt from
/// 2026-10-21 sat in the table with no reader anywhere).
///
/// Cross-tenant ON PURPOSE — this is the operator's revenue view: which workspace tried to buy
/// which plan, when, and why it did not happen. The admin console already lists every tenant
/// (`GET /api/v1/admin/tenants`), so this adds no new reach. `status` is restricted to the states
/// that mean "no money moved", so a completed purchase never appears here as a request.
pub async fn list_checkout_requests(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<CheckoutRequestQuery>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let limit = params.limit.unwrap_or(100).clamp(1, 500);

    let rows = sqlx::query(
        r#"SELECT cs.id, cs.tenant_id, cs.status, cs.amount::text AS amount, cs.currency,
                  cs.created_at, cs.metadata,
                  t.name AS tenant_name,
                  p.name AS plan_name, p.slug AS plan_slug,
                  -- Whether the workspace ALREADY holds the plan this attempt asked for. The
                  -- operator's only way to sell a plan on an install with no payment provider is
                  -- to assign it by hand (POST /api/v1/tenants/:id/plan), and until this column the
                  -- screen kept asking for work that had already been done: the row had no state
                  -- change, so a satisfied request looked identical to an ignored one.
                  EXISTS (
                      SELECT 1 FROM tenant_plan_subscriptions tps
                       WHERE tps.tenant_id = cs.tenant_id
                         AND tps.status = 'active'
                         AND tps.plan_id = cs.purchasable_id
                  ) AS plan_applied
             FROM checkout_sessions cs
             LEFT JOIN tenants t ON t.id = cs.tenant_id
             LEFT JOIN plans p
                    ON cs.purchasable_type = 'plan' AND p.id = cs.purchasable_id
            WHERE cs.status IN ('provider_missing', 'failed', 'cancelled')
            ORDER BY cs.created_at DESC
            LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let requests: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
                "tenant_id": r.try_get::<Uuid, _>("tenant_id").map(|v| v.to_string()).unwrap_or_default(),
                // A workspace row can be gone (deletion) while its attempt survives; the screen
                // renders that as a dash rather than dropping the record.
                "tenant_name": r.try_get::<Option<String>, _>("tenant_name").unwrap_or(None),
                "plan_name": r.try_get::<Option<String>, _>("plan_name").unwrap_or(None),
                "plan_slug": r.try_get::<Option<String>, _>("plan_slug").unwrap_or(None),
                "amount": r.try_get::<Option<String>, _>("amount").unwrap_or(None),
                "currency": r.try_get::<Option<String>, _>("currency").unwrap_or(None),
                "status": r.try_get::<String, _>("status").unwrap_or_default(),
                // True once the workspace holds the asked-for plan. The console renders such a row
                // as settled and hides the assign control, so the operator cannot assign the same
                // plan twice or read an already-honoured request as outstanding work.
                "plan_applied": r.try_get::<bool, _>("plan_applied").unwrap_or(false),
                "reason": r
                    .try_get::<Option<Value>, _>("metadata")
                    .unwrap_or(None)
                    .and_then(|m| m.get("reason").and_then(|v| v.as_str()).map(|s| s.to_string())),
                "created_at": r
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(
        json!({ "count": requests.len(), "requests": requests }),
    ))
}
