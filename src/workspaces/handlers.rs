use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::auth::models::Claims;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateWorkspace {
    pub name: String,
}

#[derive(Deserialize)]
pub struct RenameWorkspace {
    pub name: String,
}

fn tenant(auth: &AuthUser) -> Result<Uuid, AppError> {
    Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::Unauthorized("Invalid session".into()))
}

fn clean_name(raw: &str) -> Result<String, AppError> {
    let n = raw.trim();
    if n.is_empty() {
        return Err(AppError::Validation("A workspace needs a name.".into()));
    }
    if n.chars().count() > 120 {
        return Err(AppError::Validation(
            "That name is too long (120 characters maximum).".into(),
        ));
    }
    Ok(n.to_string())
}

/// May this session rename / delete / re-point the default workspace? The SAME rule the team
/// roster uses (`team_handler::is_workspace_owner`): the account's own administrators (its
/// creator, an appointed `company_admin`, or the platform operator) plus an impersonation
/// session. A plain member may only switch and add — never rename, delete or change the default.
async fn may_manage(state: &AppState, auth: &AuthUser) -> bool {
    if auth.is_admin || auth.role == "company_admin" || auth.impersonating.is_some() {
        return true;
    }
    let Ok(tid) = Uuid::parse_str(&auth.tenant_id) else {
        return false;
    };
    crate::handlers::team_handler::is_workspace_owner(state, auth, tid)
        .await
        .unwrap_or(false)
}

/// The portfolio businesses (ZaarHub, SwiftImpact, Giraudy Capital, SwiftSoftware) always keep
/// their account, whatever their workspace count (David's standing rule: the sister companies
/// always keep their account). Matched on the account's own name and slug.
fn protected_marker(name: &str, slug: &str) -> Option<&'static str> {
    const MARKERS: &[&str] = &[
        "funnelswift",
        "swiftsoftware",
        "swiftimpact",
        "zaarhub",
        "giraudy",
    ];
    let hay = format!("{name} {slug}").to_ascii_lowercase();
    MARKERS.iter().copied().find(|m| hay.contains(m))
}

fn row_json(r: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "name": r.get::<String, _>("name"),
        "is_default": r.get::<bool, _>("is_default"),
    })
}

/// GET /api/workspaces
pub async fn list_workspaces(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let tid = tenant(&auth)?;
    let rows = sqlx::query(
        "SELECT id, name, is_default FROM workspaces WHERE tenant_id = $1 \
         ORDER BY is_default DESC, lower(name)",
    )
    .bind(tid)
    .fetch_all(&state.db)
    .await
    .map_err(AppError::Database)?;

    let items: Vec<serde_json::Value> = rows.iter().map(row_json).collect();
    Ok(Json(json!({ "count": items.len(), "workspaces": items })))
}

/// POST /api/workspaces
pub async fn create_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateWorkspace>,
) -> AppResult<Json<serde_json::Value>> {
    let tid = tenant(&auth)?;
    let name = clean_name(&req.name)?;

    // plan ceiling, when the plan states one
    // Plan ceiling, when the plan states one. FunnelSwift sells its allowances in `plans.features`
    // (JSONB) reached through `tenant_plans`; a plan that names no `max_workspaces` is uncapped,
    // because a ceiling exists only where one was sold.
    let ceiling: Option<i64> = sqlx::query_scalar(
        "SELECT NULLIF(p.features->>'max_workspaces','')::bigint
           FROM plans p
           JOIN tenant_plan_subscriptions tp ON tp.plan_id = p.id
          WHERE tp.tenant_id = $1 AND tp.status = 'active'
          ORDER BY tp.start_date DESC
          LIMIT 1",
    )
    .bind(tid)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?
    // A plan whose blob names no `max_workspaces` returns SQL NULL — `fetch_optional` gives
    // `Some(None)` for that, and the flattened value is the "no ceiling sold" answer.
    .flatten();

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE tenant_id = $1")
        .bind(tid)
        .fetch_one(&state.db)
        .await
        .map_err(AppError::Database)?;

    if let Some(cap) = ceiling {
        if count >= cap {
            return Err(AppError::UpgradeRequired(format!(
                "Your plan includes {cap} workspace(s). Upgrade to add more."
            )));
        }
    }

    let dup: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM workspaces WHERE tenant_id = $1 AND lower(name) = lower($2)",
    )
    .bind(tid)
    .bind(&name)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;
    if dup.is_some() {
        return Err(AppError::Duplicate(
            "You already have a workspace with that name.".into(),
        ));
    }

    // the very first workspace is always the default
    let make_default = count == 0;
    let row = sqlx::query(
        "INSERT INTO workspaces (tenant_id, name, is_default) VALUES ($1, $2, $3) \
         RETURNING id, name, is_default",
    )
    .bind(tid)
    .bind(&name)
    .bind(make_default)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(json!({ "workspace": row_json(&row) })))
}

/// PATCH /api/workspaces/:id  (rename)
pub async fn rename_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<RenameWorkspace>,
) -> AppResult<Json<serde_json::Value>> {
    let tid = tenant(&auth)?;
    if !may_manage(&state, &auth).await {
        return Err(AppError::Forbidden(
            "Only the workspace owner or an admin can rename a workspace.".into(),
        ));
    }
    let name = clean_name(&req.name)?;

    let dup: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM workspaces WHERE tenant_id = $1 AND lower(name) = lower($2) AND id <> $3",
    )
    .bind(tid)
    .bind(&name)
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;
    if dup.is_some() {
        return Err(AppError::Duplicate(
            "You already have a workspace with that name.".into(),
        ));
    }

    let row = sqlx::query(
        "UPDATE workspaces SET name = $1, updated_at = now() WHERE id = $2 AND tenant_id = $3 \
         RETURNING id, name, is_default",
    )
    .bind(&name)
    .bind(id)
    .bind(tid)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("Workspace not found.".into()))?;

    Ok(Json(json!({ "workspace": row_json(&row) })))
}

/// POST /api/workspaces/:id/default
pub async fn set_default_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tid = tenant(&auth)?;
    if !may_manage(&state, &auth).await {
        return Err(AppError::Forbidden(
            "Only the workspace owner or an admin can change the default workspace.".into(),
        ));
    }

    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM workspaces WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tid)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?;
    if exists.is_none() {
        return Err(AppError::NotFound("Workspace not found.".into()));
    }

    let mut tx = state.db.begin().await.map_err(AppError::Database)?;
    sqlx::query("UPDATE workspaces SET is_default = false, updated_at = now() WHERE tenant_id = $1 AND is_default")
        .bind(tid)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    sqlx::query("UPDATE workspaces SET is_default = true, updated_at = now() WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tid)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    tx.commit().await.map_err(AppError::Database)?;

    Ok(Json(json!({ "ok": true, "default_workspace_id": id })))
}

/// DELETE /api/workspaces/:id
pub async fn delete_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tid = tenant(&auth)?;
    if !may_manage(&state, &auth).await {
        return Err(AppError::Forbidden(
            "Only the workspace owner or an admin can delete a workspace.".into(),
        ));
    }

    // Portfolio businesses are never removed, whatever their workspace count (David's standing
    // rule: the sister companies always keep their account). Matched on the account's own name and
    // slug — the SAME marker set the tenant purge door uses.
    let (tname, tslug): (String, String) =
        sqlx::query_as("SELECT name, slug FROM tenants WHERE id = $1")
            .bind(tid)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| AppError::NotFound("Workspace not found.".into()))?;
    if let Some(marker) = protected_marker(&tname, &tslug) {
        return Err(AppError::Conflict(format!(
            "This is a portfolio business ('{marker}') and its workspaces cannot be deleted."
        )));
    }

    let rows = sqlx::query("SELECT id, is_default FROM workspaces WHERE tenant_id = $1")
        .bind(tid)
        .fetch_all(&state.db)
        .await
        .map_err(AppError::Database)?;

    if rows.len() <= 1 {
        return Err(AppError::BadRequest(
            "An account must always keep at least one workspace.".into(),
        ));
    }

    let target = rows
        .iter()
        .find(|r| r.get::<Uuid, _>("id") == id)
        .ok_or_else(|| AppError::NotFound("Workspace not found.".into()))?;
    let was_default = target.get::<bool, _>("is_default");

    // Refuse deleting the workspace the caller is signed into: the session's `wsid`, or — for a
    // token minted before the switcher shipped — the account's default. Deleting the floor you are
    // standing on would strand the session; switch away first.
    let signed_in = auth
        .wsid
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|w| rows.iter().any(|r| r.get::<Uuid, _>("id") == *w))
        .or_else(|| {
            rows.iter()
                .find(|r| r.get::<bool, _>("is_default"))
                .map(|r| r.get::<Uuid, _>("id"))
        });
    if signed_in == Some(id) {
        return Err(AppError::Conflict(
            "You are signed into this workspace. Switch to a different workspace before deleting it."
                .into(),
        ));
    }

    let mut tx = state.db.begin().await.map_err(AppError::Database)?;
    sqlx::query("DELETE FROM workspaces WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tid)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

    // never leave the account without a default
    if was_default {
        sqlx::query(
            "UPDATE workspaces SET is_default = true, updated_at = now() \
             WHERE id = (SELECT id FROM workspaces WHERE tenant_id = $1 ORDER BY created_at LIMIT 1)",
        )
        .bind(tid)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    }
    tx.commit().await.map_err(AppError::Database)?;

    Ok(Json(json!({ "ok": true, "deleted": id })))
}

/// POST /api/v1/workspaces/:id/switch — move this session's active workspace.
///
/// The switcher re-mints the session token with `wsid` set to the target workspace, reusing the
/// SAME token shape the login/register handlers use — no second auth route. The caller may only
/// switch to a workspace inside their OWN account (`tenant_id`); any other id answers 404. The
/// session keeps its subject, role and account — only the workspace the console shows changes.
pub async fn switch_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tid = tenant(&auth)?;

    let row =
        sqlx::query("SELECT id, name, is_default FROM workspaces WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tid)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| AppError::NotFound("Workspace not found.".into()))?;

    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: auth.user_id.clone(),
        tenant_id: auth.tenant_id.clone(),
        email: auth.email.clone(),
        role: auth.role.clone(),
        exp: now + 86400 * 30,
        iat: now,
        aud: Some("funnelswift-api".to_string()),
        iss: Some("funnelswift".to_string()),
        impersonating: auth.impersonating.clone(),
        wsid: Some(id.to_string()),
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))?;

    Ok(Json(json!({
        "token": token,
        "workspace": row_json(&row),
    })))
}
