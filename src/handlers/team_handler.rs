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

    // WHAT A CUSTOMER MAY HAND OUT — checked FIRST, before any allowance (2026-10-31).
    // `users.role` carries BOTH responsibilities here: `admin` is the PLATFORM operator, and
    // `auth::middleware` sets `is_admin` from exactly that string, which is what opens every
    // `/api/v1/admin/*` handler. The shared mint accepts `admin` because the OPERATOR's console
    // legitimately appoints platform admins — so without this gate a workspace owner could invite
    // a person as `admin` and hand out platform-wide authority over every other customer. The
    // console never offered it as a sensible choice, but the API is the door that has to refuse it.
    //
    // Order matters: the role is parsed BEFORE `enforce_feature_limit`, so the dangerous value can
    // never reach the mint whatever a plan's seat allowance happens to be now or later. (Measured
    // 2026-10-31: every plan on this install carries `max_team_members = 1`, so the allowance arm
    // answered 402 first and the role was never looked at — the escalation was latent, not absent.)
    let role = customer_role(req.get("role").and_then(|v| v.as_str()))?;

    // The plan decides next. `max_team_members` counts the workspace's ACTIVE people, the owner
    // included, so a single-seat plan refuses here instead of creating a login nobody may use.
    // The refusal is put through [`seat_refusal`] first: if no plan on this installation sells more
    // seats than the caller already has, "Upgrade to increase your limit" is a dead end and must not
    // be said (measured 2026-11-05 on Agency / Scale, the top plan).
    if let Err(refusal) =
        features::enforce_feature_limit(&state, tenant_id, "max_team_members", "Team members").await
    {
        return Err(seat_refusal(&state, tenant_id, refusal).await);
    }

    let mut body = req.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.insert("role".to_string(), json!(role));
    }
    mint_workspace_user(&state, tenant_id, &body).await
}

/// The fleet's generic limit sentence ends "Upgrade to increase your limit." — a DEAD END when the
/// workspace already holds the largest seat allowance any plan on this installation sells.
///
/// Measured 2026-11-05: every plan, up to Agency / Scale at the top of the price list, carries
/// `max_team_members = 1`, so a customer paying for the best plan asked to add a colleague and was
/// told to upgrade to something that does not exist (`402 Team members limit reached (1/1).
/// Upgrade to increase your limit.`). Nothing on the screen could satisfy that instruction.
///
/// So when no plan offers more seats than the caller already has, say what is true instead: the
/// allowance is in use and a person has to raise it. When SOME plan does offer more, the generic
/// sentence is correct and is returned untouched — this can never suppress a real upgrade path, and
/// it needs no change the day a plan's seat count is raised.
async fn seat_refusal(state: &AppState, tenant_id: Uuid, refusal: AppError) -> AppError {
    // The number the gates actually enforce for THIS workspace (tenant override, plan row or plan
    // column — the one resolver), not the plan's raw column.
    let current = match features::resolved_limit(state, tenant_id, "max_team_members").await {
        Ok(Some(limit)) if limit > 0 => limit,
        _ => return refusal,
    };
    let best: Option<i32> =
        sqlx::query_scalar("SELECT MAX(max_team_members) FROM plans WHERE max_team_members > 0")
            .fetch_one(&state.pool)
            .await
            .ok()
            .flatten();
    if matches!(best, Some(best) if best > current) {
        return refusal;
    }
    AppError::UpgradeRequired(format!(
        "Your plan includes {current} teammate{}. That seat is already in use, and no plan on this \
         installation includes more, so there is nothing to upgrade to yet. Send us a message at {} \
         and we will add the seats for you.",
        if current == 1 { "" } else { "s" },
        crate::email::SUPPORT_CONTACT
    ))
}

/// The roles a WORKSPACE OWNER may hand out — the operator-only `admin` (platform authority) is not
/// among them, and saying so is a 400 that names the field rather than a silent downgrade: a caller
/// who asked for `admin` must not be told "added" while a member row was written.
///
/// `company_admin` and `business_user` are workspace roles; `user` is the member default. The
/// vocabulary comes from [`teammate_role`] (the shared parse and the one list), and the refusal is
/// this door's own, so a role the operator console accepts is still refused HERE.
fn customer_role(raw: Option<&str>) -> Result<String, AppError> {
    let role = teammate_role(raw)?;
    if role == "admin" {
        return Err(AppError::BadRequest(
            "role 'admin' is a platform role, not a workspace one — choose user, company_admin or \
             business_user"
                .into(),
        ));
    }
    Ok(role)
}

#[cfg(test)]
mod tests {
    use super::customer_role;

    /// The escalation this gate exists for: a workspace owner may not mint a platform admin.
    #[test]
    fn a_customer_may_not_hand_out_the_platform_role() {
        let err =
            customer_role(Some("admin")).expect_err("admin must be refused on the customer door");
        let text = format!("{err:?}");
        assert!(
            text.contains("platform role"),
            "refusal must name the reason: {text}"
        );
    }

    /// The three roles a workspace may use still pass, unchanged — the gate is narrow on purpose.
    #[test]
    fn the_workspace_roles_still_pass() {
        for r in ["user", "company_admin", "business_user"] {
            assert_eq!(customer_role(Some(r)).unwrap(), r);
        }
        // absent/blank keeps the member default, exactly as the shared parse does
        assert_eq!(customer_role(None).unwrap(), "user");
        assert_eq!(customer_role(Some("  ")).unwrap(), "user");
    }

    /// A word outside the shared vocabulary is still the shared 400 (not this door's new one).
    #[test]
    fn an_unknown_role_is_still_refused_by_name() {
        let err = customer_role(Some("superuser")).expect_err("unknown role must be refused");
        assert!(format!("{err:?}").contains("role must be one of"));
    }
}
