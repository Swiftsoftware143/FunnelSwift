//! Per-account API keys — the fleet-standard mint surface (kanban t_164fbcc3, plan
//! `/opt/swift/docs/fleet-per-account-keys-and-mcp-2026-11-25.md` §P1).
//!
//! WHY THIS MODULE EXISTS when `handlers::api_keys` already mints keys: the fleet is standardising
//! on ONE surface every product serves at the same five paths, so a principal (the admin, a
//! portfolio CEO, an MCP client) can be provisioned the same way in every app:
//!
//! ```text
//!   GET    /api/personal-api-keys              list this account's keys (prefix only)
//!   POST   /api/personal-api-keys              mint one  -> plaintext returned once
//!   DELETE /api/personal-api-keys/:id          revoke one (soft: `revoked_at`)
//!   POST   /api/personal-api-keys/:id/rotate   replace one with a fresh secret
//!   POST   /api/personal-api-keys/:id/reveal   show it again (caller's password required)
//! ```
//!
//! It is a THIN DOOR, not a second implementation. Every arm delegates to
//! [`crate::handlers::api_keys`], which owns the single writer (`mint_on`: `fs_` secret, sha256
//! `key_hash`, sealed `key_sealed`, `activity_log` audit) and the bearer resolver the MCP door and
//! the REST boundary already use — so a key minted here authenticates at `/mcp` on the very next
//! call, and the two surfaces can never drift in hashing, sealing, prefix or audit.
//!
//! Response shape is CoreSwift's (`id`, `name`, `key`, `prefix`) PLUS FunnelSwift's own field names
//! (`token`, `key_prefix`), because the existing Settings screen and the MCP card read those.
//!
//! # Tenancy
//!
//! A key resolves to exactly ONE account and never to anything the caller names in a body. Four of
//! the five arms are bound to the CALLER's own tenant by [`own_tenant`]; there is no id in the path
//! that could be spoofed and every statement filters on it, so a foreign key id answers 404.
//!
//! The ONE exception is `POST` with `account_id`, which exists because P3 provisions principals
//! that have not signed in: a platform admin (`users.role = 'admin'`, `AuthUser::is_admin`) may mint
//! for another account. Two guards make that safe rather than a privilege hole:
//!
//!   * only `is_admin` may name `account_id` — anyone else gets 403, never a silently-ignored field;
//!   * the minted row's `created_by` is the TARGET account's own owner, never the admin. The bearer
//!     resolver reads the creator's LIVE role on every request, so recording the admin there would
//!     hand a probe/customer account a platform-admin credential. An account with no user yet gets
//!     `created_by = NULL`, which resolves to the plain `user` role.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::handlers::api_keys as inner;
use crate::state::AppState;

fn own_tenant(auth: &AuthUser) -> Result<Uuid, AppError> {
    Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::Unauthorized("Invalid tenant in token".to_string()))
}

fn caller_id(auth: &AuthUser) -> Option<Uuid> {
    Uuid::parse_str(&auth.user_id).ok()
}

/// Both field names for every mint response: `key`/`prefix` (fleet standard, CoreSwift's shape) and
/// `token`/`key_prefix` (FunnelSwift's own — the Settings screen and the MCP card read these).
fn mint_body(id: &str, name: &str, token: &str, prefix: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "key": token,
        "token": token,
        "prefix": prefix,
        "key_prefix": prefix,
        "note": "This key can be viewed again (POST /api/personal-api-keys/:id/reveal) with your password.",
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub name: Option<String>,
    /// Platform admins only: mint into ANOTHER account (`tenants.id`). A customer sending this is
    /// refused 403 rather than silently ignored — a typo must never mint a credential elsewhere.
    pub account_id: Option<String>,
}

/// `POST /api/personal-api-keys` — mint. `201` with the plaintext once.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let Some(account_id) = req.account_id.as_deref() else {
        // Own account: the shipped door, unchanged (one writer, one audit trail).
        let (status, Json(body)) = inner::create_api_key(
            State(state),
            auth,
            Json(inner::CreateApiKeyRequest { name: req.name }),
        )
        .await?;
        let token = body
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prefix = body
            .get("key_prefix")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let name = body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Ok((status, Json(mint_body(&id, &name, &token, &prefix))));
    };

    if !auth.is_admin {
        return Err(AppError::Forbidden(
            "Only a platform admin may mint a key for another account".to_string(),
        ));
    }
    let target = Uuid::parse_str(account_id.trim())
        .map_err(|_| AppError::BadRequest("account_id must be a uuid".to_string()))?;
    let account: Option<(String, String)> =
        sqlx::query_as("SELECT name, status FROM tenants WHERE id = $1")
            .bind(target)
            .fetch_optional(&state.pool)
            .await?;
    let Some((account_name, status)) = account else {
        return Err(AppError::NotFound("Account not found".to_string()));
    };
    if status != "active" {
        return Err(AppError::Conflict(format!(
            "'{account_name}' is {status}, not active — a key minted for it would not authenticate"
        )));
    }

    let name = req
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "API key".to_string());
    crate::security::display_name::check_len(&name, 255, "name")
        .map_err(AppError::UnprocessableEntity)?;

    // The target account's OWN owner minted it (see the module note): a key carries its creator's
    // live role on every request, so the platform admin's identity must never be baked into a
    // customer's credential.
    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE tenant_id = $1 AND is_active = true \
         ORDER BY (role = 'company_admin') DESC, created_at LIMIT 1",
    )
    .bind(target)
    .fetch_optional(&state.pool)
    .await?;

    let mut tx = state.pool.begin().await?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM api_keys WHERE tenant_id = $1 AND revoked_at IS NULL LIMIT 1",
    )
    .bind(target)
    .fetch_optional(&mut *tx)
    .await?;
    if existing.is_some() {
        return Err(AppError::Conflict(format!(
            "'{account_name}' already holds a live API key — rotate that one instead of minting a second"
        )));
    }
    let minted = inner::mint_on(&mut tx, target, owner, &name).await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(mint_body(
            &minted.id.to_string(),
            &minted.name,
            &minted.token,
            &minted.prefix,
        )),
    ))
}

/// `GET /api/personal-api-keys` — this account's keys, prefix only, never the plaintext.
pub async fn list(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Value>> {
    let Json(body) = inner::list_api_keys(State(state), auth).await?;
    let rows = body
        .get("api_keys")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let keys: Vec<Value> = rows
        .iter()
        .map(|r| {
            let prefix = r.get("key_prefix").cloned().unwrap_or(Value::Null);
            let revoked = r.get("revoked_at").cloned().unwrap_or(Value::Null);
            json!({
                "id": r.get("id").cloned().unwrap_or(Value::Null),
                "name": r.get("name").cloned().unwrap_or(Value::Null),
                "key_prefix": prefix.clone(),
                "prefix": prefix,
                "is_active": revoked.is_null(),
                "created_at": r.get("created_at").cloned().unwrap_or(Value::Null),
                "last_used_at": r.get("last_used_at").cloned().unwrap_or(Value::Null),
                "revoked_at": revoked,
                "recoverable": r.get("recoverable").cloned().unwrap_or(Value::Bool(false)),
            })
        })
        .collect();
    Ok(Json(json!({ "keys": keys })))
}

/// `DELETE /api/personal-api-keys/:id` — revoke (soft; the row stays for audit).
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let Json(mut body) = inner::revoke_api_key(State(state), auth, Path(id)).await?;
    if let Value::Object(map) = &mut body {
        map.insert("revoked".to_string(), Value::Bool(true));
    }
    Ok(Json(body))
}

/// `POST /api/personal-api-keys/:id/rotate` — one atomic revoke-and-issue; the old secret dies with
/// the same commit that stores the new one.
pub async fn rotate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<Value>)> {
    // The caller's own tenant is enforced inside the delegated handler; this id lookup only decides
    // whether the row is even visible, and a foreign id answers the app's 404.
    let tenant = own_tenant(&auth)?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM api_keys WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let (status, Json(body)) = inner::regenerate_api_key(State(state), auth, Path(id)).await?;
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let prefix = body
        .get("key_prefix")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let new_id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut out = mint_body(&new_id, &name, &token, &prefix);
    if let Value::Object(map) = &mut out {
        map.insert("rotated_from".to_string(), Value::String(id.to_string()));
    }
    Ok((status, Json(out)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevealRequest {
    /// The caller's CURRENT password — a stolen session must not be enough to read a live credential.
    pub password: String,
}

/// `POST /api/personal-api-keys/:id/reveal` — show the key again. Password re-auth and an
/// `activity_log` audit row are the delegated handler's (`api_key_revealed`).
pub async fn reveal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<RevealRequest>,
) -> AppResult<Json<Value>> {
    let _ = caller_id(&auth);
    let Json(body) = inner::reveal_api_key(
        State(state),
        auth,
        Path(id),
        Json(inner::RevealKeyRequest {
            password: req.password,
        }),
    )
    .await?;
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let prefix = body
        .get("key_prefix")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(Json(json!({
        "id": id.to_string(),
        "key": token,
        "token": token,
        "prefix": prefix,
        "key_prefix": prefix,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two field-name families must always agree: the MCP card reads `token`/`key_prefix`, the
    /// fleet's provisioning step reads `key`/`prefix`. A body that carries one and not the other is
    /// how a caller silently gets an empty credential.
    #[test]
    fn mint_body_carries_both_field_families() {
        let body = mint_body("id-1", "MCP — ceo", "fs_abc", "fs_abc12345");
        assert_eq!(body["key"], "fs_abc");
        assert_eq!(body["token"], "fs_abc");
        assert_eq!(body["prefix"], "fs_abc12345");
        assert_eq!(body["key_prefix"], "fs_abc12345");
        assert_eq!(body["name"], "MCP — ceo");
    }

    #[test]
    fn create_request_refuses_a_typo_instead_of_minting_locally() {
        // `account` (the typo) must not deserialize: silently minting for the CALLER's account while
        // an admin believed it minted for a customer is the failure this guard exists for.
        let ok: Result<CreateRequest, _> = serde_json::from_str(r#"{"name":"x","account_id":"a"}"#);
        assert!(ok.is_ok());
        let typo: Result<CreateRequest, _> = serde_json::from_str(r#"{"name":"x","account":"a"}"#);
        assert!(typo.is_err());
    }
}
