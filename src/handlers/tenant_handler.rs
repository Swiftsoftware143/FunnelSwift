use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::handlers::plan_handler::set_active_plan;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

/// Tenant slug shape used by every other provisioning site in the app
/// (`admin_handler.rs:70`, `auth/handlers.rs`, `public_signup_handler.rs`):
/// lowercased, spaces to dashes. `tenants.slug` is NOT NULL + UNIQUE.
fn slugify(name: &str) -> String {
    let slug = name.trim().to_lowercase().replace(' ', "-");
    slug.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// `tenants.status` (migration 053, NOT NULL DEFAULT 'active') — the tenant/workspace lifecycle
/// flag the admin screen's Status column reads and its edit modal writes. The screen offers exactly
/// two values (`www/dashboard.js` ETN: Active / Inactive), so anything else is rejected here as a
/// 400 instead of reaching Postgres and coming back as a 23514 from `tenants_status_check`.
fn parse_status(raw: &str) -> AppResult<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "active" => Ok("active"),
        "inactive" => Ok("inactive"),
        other => Err(AppError::BadRequest(format!(
            "Invalid status '{other}': expected 'active' or 'inactive'"
        ))),
    }
}

/// The role that owns the lifecycle flag: `auth::middleware::AuthUser::is_admin` is exactly
/// `role == "admin"`, and the Tenants screen that writes `status` is admin-gated.
pub const PLATFORM_ADMIN_ROLE: &str = "admin";

/// The ONE enforcement reader of `tenants.status` (t_af890bbf). Every gate in the app reads the
/// flag through this function so the "what does inactive mean" answer has a single home.
///
/// `Ok(Some("active"))` = live workspace, `Ok(Some("inactive"))` = retired, `Ok(None)` = the
/// tenant row itself is gone (a deleted workspace whose 30-day JWT is still in the wild, since
/// JWTs are stateless). Callers treat anything that is not `active` as "not live" — fail closed.
pub async fn tenant_status_for(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("SELECT status FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

/// May this identity still USE its workspace? This is the decision of t_af890bbf, applied by the
/// access surfaces only:
///
/// * ENFORCED — `auth::handlers::login`, `forgot_password`, `reset_password`, and the central
///   `auth::global_auth::require_auth` gate (which is what ends an existing 30-day session), plus
///   `admin_handler::impersonate` refusing an inactive *target*.
/// * NOT ENFORCED, on purpose — public card serving/tracking/lead capture. Kinetic cards are
///   physical artifacts with QR codes already in customers' hands and are addressed by third
///   parties, so a mis-ticked dropdown would silently break a live campaign that nobody can log in
///   to notice. Documented in docs/ADMIN_GUIDE.md ("Workspace Status").
///
/// `role == PLATFORM_ADMIN_ROLE` is exempt on purpose: the console that writes `status` is itself
/// a tenant, so enforcing the flag on the operator would make a mis-ticked workspace — including
/// the operator's own — impossible to switch back from the console. Tenant-level admins
/// (`company_admin`) are NOT exempt; they are the customer.
pub async fn workspace_access_allowed(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    role: &str,
) -> Result<bool, sqlx::Error> {
    if role == PLATFORM_ADMIN_ROLE {
        return Ok(true);
    }
    Ok(tenant_status_for(pool, tenant_id).await?.as_deref() == Some("active"))
}

pub async fn list_tenants(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    // NOTE: tenants never had status/is_visible/plan_id columns, so the old query referenced
    // columns that do not exist and then swallowed the resulting error with
    // `.unwrap_or_default()` — every caller silently received `[]` (126 rows in the DB, empty
    // list in the UI). `email` exists since migration 052 and `status` since 053 (both written by
    // the served admin screen), and the plan comes from the newest active subscription row.
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            chrono::NaiveDateTime,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        ),
    >(
        r#"SELECT t.id, t.name, t.slug, t.created_at, p.name AS plan_name, p.slug AS plan_slug, t.email, t.status
           FROM tenants t
           LEFT JOIN LATERAL (
               SELECT plan_id FROM tenant_plan_subscriptions s
               WHERE s.tenant_id = t.id AND s.status = 'active'
               ORDER BY s.start_date DESC NULLS LAST LIMIT 1
           ) sub ON TRUE
           LEFT JOIN plans p ON p.id = sub.plan_id
           ORDER BY t.created_at DESC"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let tenants: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "name": r.1, "slug": r.2, "created_at": r.3,
                "plan_name": r.4, "plan_slug": r.5, "plan": r.4, "status": r.7,
                "email": r.6
            })
        })
        .collect();
    Ok(Json(json!(tenants)))
}

pub async fn get_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    // The served admin screen's Edit modal opens with `api("/api/v1/tenants/"+id)` and prefills
    // name/email/slug/status from the response (`www/dashboard.js` ETN -> RTN). This route used to
    // answer `{"id": id}` and nothing else, so every field came back undefined: the modal showed an
    // empty form and always re-selected "Active". Without a real read here the Status column could
    // not round-trip even once the column existed — an "inactive" row edited through the screen was
    // silently switched back to active on save.
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            chrono::NaiveDateTime,
            Option<String>,
            Option<String>,
            String,
        ),
    >(
        r#"SELECT t.id, t.name, t.slug, t.email, t.created_at, p.name AS plan_name, p.slug AS plan_slug, t.status
           FROM tenants t
           LEFT JOIN LATERAL (
               SELECT plan_id FROM tenant_plan_subscriptions s
               WHERE s.tenant_id = t.id AND s.status = 'active'
               ORDER BY s.start_date DESC NULLS LAST LIMIT 1
           ) sub ON TRUE
           LEFT JOIN plans p ON p.id = sub.plan_id
           WHERE t.id = $1"#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tenant not found".into()))?;
    Ok(Json(json!({
        "id": row.0, "name": row.1, "slug": row.2, "email": row.3, "created_at": row.4,
        "plan_name": row.5, "plan_slug": row.6, "plan": row.5,
        "status": row.7
    })))
}

pub async fn create_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let id = Uuid::new_v4();
    let name = req["name"].as_str().unwrap_or("New Tenant");
    // The served admin screen (www/dashboard.js `STN`, www-app, www-admin) posts
    // {name, email, slug, status}. `email` is a real nullable column since migration 052
    // (the statement used to name a column that never existed -> ERROR 42703, so no tenant
    // could be created from that screen at all).
    let email = req.get("email").and_then(|v| v.as_str());
    // The same screen posts {name, email, slug, status} (`www/dashboard.js` STN). `status` is a
    // real column since migration 053; absent means the column default, i.e. the "active" the list
    // used to hard-code.
    let status = match req.get("status").and_then(|v| v.as_str()) {
        Some(raw) => parse_status(raw)?,
        None => "active",
    };
    // `slug` is NOT NULL + UNIQUE and the old statement omitted it entirely — a second 500
    // sitting behind the 42703. Callers may send one; otherwise derive it from the name,
    // like every other tenant-provisioning site in the app.
    let slug = match req
        .get("slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => slugify(name),
    };
    let slug = if slug.is_empty() {
        format!("tenant-{}", &id.simple().to_string()[..8])
    } else {
        slug
    };
    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.pool)
        .await?;
    if taken > 0 {
        return Err(AppError::Conflict(format!(
            "A tenant with slug '{slug}' already exists"
        )));
    }
    sqlx::query("INSERT INTO tenants (id, name, slug, email, status) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(name)
        .bind(&slug)
        .bind(email)
        .bind(status)
        .execute(&state.pool)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": id, "name": name, "slug": slug, "email": email, "status": status, "message": "Tenant created"}),
        ),
    ))
}

pub async fn update_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let name = req
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let email = req.get("email").and_then(|v| v.as_str());
    // Absent status leaves the column alone (COALESCE below); a present one is validated first so a
    // bad value is a 400 and never a 23514 from `tenants_status_check`.
    let status = match req.get("status").and_then(|v| v.as_str()) {
        Some(raw) => Some(parse_status(raw)?),
        None => None,
    };
    let slug = req
        .get("slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // slug is UNIQUE: check before writing, so a duplicate is a 409 and not a 500.
    if let Some(s) = slug {
        let taken: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE slug = $1 AND id <> $2")
                .bind(s)
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
        if taken > 0 {
            return Err(AppError::Conflict(format!(
                "A tenant with slug '{s}' already exists"
            )));
        }
    }
    // email = tenants.email (migration 052). `plan_id` is NOT a tenants column — the statement
    // used to raise 42703 for it, which is why the UI's "Assign Plan" button could never save;
    // the plan is the active tenant_plan_subscriptions row (see `set_active_plan`).
    let rows = sqlx::query(
        "UPDATE tenants SET name=COALESCE($2, name), email=COALESCE($3, email), slug=COALESCE($4, slug), status=COALESCE($5, status) WHERE id=$1",
    )
    .bind(id)
    .bind(name)
    .bind(email)
    .bind(slug)
    .bind(status)
    .execute(&state.pool)
    .await?;
    if rows.rows_affected() == 0 {
        return Err(AppError::NotFound("Tenant not found".into()));
    }
    // The same screen PUTs {plan_id} to this endpoint (www/dashboard.js SET -> PUT /tenants/:id).
    if let Some(raw) = req
        .get("plan_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        let plan_id = Uuid::parse_str(raw)
            .map_err(|_| AppError::BadRequest("Valid plan_id is required".into()))?;
        set_active_plan(&state.pool, id, plan_id).await?;
    }
    Ok(Json(json!({"message": "Tenant updated"})))
}

pub async fn delete_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    sqlx::query("DELETE FROM tenants WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Tenant deleted"})))
}

pub async fn assign_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let plan_id = req
        .get("plan_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("Valid plan_id is required".into()))?;
    // `tenants.plan_id` does not exist — the statement used to raise ERROR 42703 for every call.
    // The plan is the active tenant_plan_subscriptions row, the same row the tenant list reads
    // and `plan_handler::admin_assign_plan` writes; `set_active_plan` is the only writer.
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if exists == 0 {
        return Err(AppError::NotFound("Tenant not found".into()));
    }
    set_active_plan(&state.pool, id, plan_id).await?;
    Ok(Json(json!({"message": "Plan assigned"})))
}
