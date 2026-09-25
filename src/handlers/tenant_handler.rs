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

pub async fn list_tenants(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    // NOTE: tenants still has no status/is_visible/plan_id columns, so the old query referenced
    // columns that do not exist and then swallowed the resulting error with
    // `.unwrap_or_default()` — every caller silently received `[]` (124 rows in the DB, empty
    // list in the UI). `email` now exists (migration 052), the plan comes from the newest active
    // subscription row, and `status` is reported as the literal the UI already defaults to.
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
        ),
    >(
        r#"SELECT t.id, t.name, t.slug, t.created_at, p.name AS plan_name, p.slug AS plan_slug, t.email
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
                "plan_name": r.4, "plan_slug": r.5, "plan": r.4, "status": "active",
                "email": r.6
            })
        })
        .collect();
    Ok(Json(json!(tenants)))
}

pub async fn get_tenant(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!({"id": id})))
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
    sqlx::query("INSERT INTO tenants (id, name, slug, email) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(name)
        .bind(&slug)
        .bind(email)
        .execute(&state.pool)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": id, "name": name, "slug": slug, "email": email, "message": "Tenant created"}),
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
        "UPDATE tenants SET name=COALESCE($2, name), email=COALESCE($3, email), slug=COALESCE($4, slug) WHERE id=$1",
    )
    .bind(id)
    .bind(name)
    .bind(email)
    .bind(slug)
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
