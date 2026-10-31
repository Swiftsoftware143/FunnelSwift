//! Customer-facing teammate invitations — `POST /api/v1/team/invite` (2026-10-31).
//!
//! Until now a FunnelSwift workspace could only grow if the PLATFORM OPERATOR added the person by
//! hand (`/api/v1/admin/users`): the customer had no door at all, so the plan's `max_team_members`
//! allowance the operator console has shown since 2026-10-31 bounded nothing — there was no way to
//! spend it. This handler is that door.
//!
//! Three rules live here, each one before anything is created:
//!   1. AUTHORITY — the caller must administer THIS workspace. The tenant comes from the session,
//!      never from the body, so a session cannot mint into somebody else's workspace.
//!   2. ALLOWANCE — the plan's `max_team_members` is checked FIRST, because the alternative is a
//!      user row plus a credentials mail for a login the plan does not allow (the "account exists,
//!      nobody may use it" failure this fleet has already had once).
//!   3. VOCABULARY — the role is parsed against the fleet's four workspace roles before the row is
//!      written, so a typo is a 400 that names the field instead of a user no gate recognises.
//!
//! WHO IS A WORKSPACE ADMIN — measured live 2026-10-31, and it is the reason this is not a plain
//! `role ==` test. `users.role` carries BOTH responsibilities in this app: `admin` is the PLATFORM
//! operator (`middleware.rs` sets `is_admin` from exactly that string, which is what opens
//! `/api/v1/admin/*`), while a workspace's own administrator is `company_admin`. And the public
//! signup mints its first user as plain `user` — so a self-serve workspace has NO role that marks
//! its owner at all. Promoting that first user would be a role-model change with platform
//! consequences (an `admin` row is a platform admin), so the door recognises the workspace's
//! CREATOR instead: the oldest active person in the workspace is the account the signup made, and
//! an operator can also appoint `company_admin` explicitly. Nobody else may add people.
//!
//! The mint itself is NOT re-implemented: `admin_handler::mint_workspace_user` is the same code the
//! operator console runs, so the address rule, the duplicate check, the generated password, the
//! credentials mail and the withheld/handover contract cannot drift between the two doors.

use axum::{extract::State, Json};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::handlers::admin_handler::{mint_workspace_user, teammate_role};
use crate::state::AppState;

/// True when `user_id` is the workspace's creator — its oldest active person, which is the row the
/// public signup wrote. Read from the database rather than from a token claim: a claim can outlive
/// the row it described (this app has already had a 30-day token outrank its own row).
async fn is_workspace_creator(state: &AppState, tenant_id: Uuid, user_id: Uuid) -> AppResult<bool> {
    let oldest: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE tenant_id = $1 AND is_active = true \
         ORDER BY created_at, id LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(oldest == Some(user_id))
}

/// Who may add somebody to a workspace: its creator or an appointed `company_admin`. The platform
/// operator passes too (a session can be an impersonation of the operator's own account), but the
/// door it normally uses is `/api/v1/admin/users`.
async fn may_invite(state: &AppState, auth: &AuthUser, tenant_id: Uuid) -> AppResult<bool> {
    if auth.is_admin || auth.role == "company_admin" {
        return Ok(true);
    }
    let user_id = Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user".into()))?;
    is_workspace_creator(state, tenant_id, user_id).await
}

pub async fn invite_teammate(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::Unauthorized("Invalid tenant".into()))?;

    if !may_invite(&state, &auth, tenant_id).await? {
        return Err(AppError::Forbidden(
            "Only the workspace owner or a company admin can add a teammate".into(),
        ));
    }

    // The plan decides next. `max_team_members` counts the workspace's ACTIVE people, the owner
    // included, so a single-seat plan refuses here with the fleet's own upgrade sentence (402)
    // instead of creating a login nobody may use.
    features::enforce_feature_limit(&state, tenant_id, "max_team_members", "Team members").await?;

    // Parse the role before the mint so an unknown role never reaches a row.
    let role = teammate_role(req.get("role").and_then(|v| v.as_str()))?;
    let mut body = req.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.insert("role".to_string(), json!(role));
    }
    mint_workspace_user(&state, tenant_id, &body).await
}
