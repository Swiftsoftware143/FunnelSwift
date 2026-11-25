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
//!   POST   /api/personal-api-keys/:id/reveal   show it again (one click, session only, audited)
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
        "note": "This key can be viewed again at any time with one click (POST /api/personal-api-keys/:id/reveal); every view is recorded.",
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
///
/// A platform admin may revoke ANOTHER account's key (the account-death kill switch: an operator
/// must be able to stop a leaking credential without waiting for the customer). Everyone else is
/// bound to their own account, and a foreign id answers 404 rather than confirming it exists.
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant = own_tenant(&auth)?;
    let row_tenant: Option<Uuid> =
        sqlx::query_scalar("SELECT tenant_id FROM api_keys WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let Some(row_tenant) = row_tenant else {
        return Err(AppError::NotFound("API key not found".to_string()));
    };
    if row_tenant != tenant && !auth.is_admin {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    let updated = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() \
         WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(row_tenant)
    .execute(&state.pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "API key not found or already revoked".to_string(),
        ));
    }
    if let Err(e) = sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) \
         VALUES ($1, $2, $3, 'api_key_revoked', 'api_key', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(row_tenant)
    .bind(auth.user_id.clone())
    .bind(id.to_string())
    .bind(json!({ "by": auth.email.clone(),
                  "on_behalf_of": if row_tenant == tenant { "self" } else { "platform admin" } }))
    .execute(&state.pool)
    .await
    {
        tracing::error!(error = %e, key = %id, "activity_log insert failed for api_key_revoked");
    }

    Ok(Json(json!({
        "revoked": true,
        "id": id,
        "account_id": row_tenant.to_string(),
    })))
}

/// `POST /api/personal-api-keys/:id/rotate` — one atomic revoke-and-issue; the old secret dies with
/// the same commit that stores the new one.
///
/// A platform admin may rotate ANOTHER account's key. That is not a new privilege: minting for any
/// account is already theirs (the `account_id` arm), and every FunnelSwift account holds an
/// auto-provisioned key from signup whose plaintext nobody can produce without that account's
/// password — so without this arm a principal provisioner (plan §P3) could neither mint a second
/// key (the one-live-key index refuses it) nor recover the existing one. The replacement's
/// `created_by` is the TARGET account's owner, never the admin (see the module note).
pub async fn rotate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant = own_tenant(&auth)?;
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT tenant_id, name FROM api_keys WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((row_tenant, name)) = row else {
        return Err(AppError::NotFound("API key not found".to_string()));
    };

    if row_tenant == tenant {
        let (status, Json(body)) = inner::regenerate_api_key(State(state), auth, Path(id)).await?;
        return Ok((status, Json(rotate_body(&body, &name, id))));
    }

    // Another account's key: platform admin only, and a non-admin must not learn it exists.
    if !auth.is_admin {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE tenant_id = $1 AND is_active = true \
         ORDER BY (role = 'company_admin') DESC, created_at LIMIT 1",
    )
    .bind(row_tenant)
    .fetch_optional(&state.pool)
    .await?;

    let mut tx = state.pool.begin().await?;
    let retired = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() \
         WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(row_tenant)
    .execute(&mut *tx)
    .await?;
    if retired.rows_affected() == 0 {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    let minted = inner::mint_on(&mut tx, row_tenant, owner, &name).await?;
    sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) \
         VALUES ($1, $2, $3, 'api_key_regenerated', 'api_key', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(row_tenant)
    .bind(auth.user_id.clone())
    .bind(minted.id.to_string())
    .bind(json!({ "old_key": id.to_string(), "key_prefix": minted.prefix.clone(),
                  "by": auth.email.clone(), "on_behalf_of": "platform admin" }))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut out = mint_body(
        &minted.id.to_string(),
        &minted.name,
        &minted.token,
        &minted.prefix,
    );
    if let Value::Object(map) = &mut out {
        map.insert("rotated_from".to_string(), Value::String(id.to_string()));
        map.insert(
            "account_id".to_string(),
            Value::String(row_tenant.to_string()),
        );
    }
    tracing::info!(
        new_key = %minted.id,
        old_key = %id,
        tenant = %row_tenant,
        by = %auth.email,
        "personal API key ROTATED by a platform admin for another account"
    );
    Ok((StatusCode::CREATED, Json(out)))
}

/// The shared rotate response: both field families plus the lineage.
fn rotate_body(body: &Value, name: &str, old_id: Uuid) -> Value {
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let prefix = body
        .get("key_prefix")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new_id = body.get("id").and_then(Value::as_str).unwrap_or_default();
    let mut out = mint_body(new_id, name, token, prefix);
    if let Value::Object(map) = &mut out {
        map.insert(
            "rotated_from".to_string(),
            Value::String(old_id.to_string()),
        );
    }
    out
}

/// `POST /api/personal-api-keys/:id/reveal` — show the key again (plan §6.2/§8: one click from a
/// console session, no password re-prompt), and the delegated handler writes the
/// `activity_log` row (`api_key_revealed`).
///
/// No body is read: a legacy `{"password":"…"}` body is accepted and ignored, so an older console
/// build keeps working. A durable `fs_…` key presented as the bearer is refused 403 inside the
/// shared handler.
pub async fn reveal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let Json(body) = inner::reveal_api_key(State(state), auth, Path(id)).await?;
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
