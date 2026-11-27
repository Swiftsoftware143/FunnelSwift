//! API keys — the durable bearer credential FunnelSwift issues to external callers
//! (kanban t_7863696b).
//!
//! Protected routes are JWT-only and the login token lives 30 days, but a cross-app
//! connector — the fleet MCP server — needs a credential it can paste once and keep.
//! A key is minted here, stored twice, and the bearer resolver in `auth::global_auth` /
//! `auth::middleware` accepts it as `Authorization: Bearer *** on any tenant route.
//!
//! Two copies, two jobs (David, 2026-11-11 — withdrawing "shown once, never again"):
//!   * `key_hash`   — hex SHA-256, the ONLY thing `resolve_api_key` reads;
//!   * `key_sealed` — the same key sealed with this app's provider-key envelope
//!     (`enc:v1:…`), used ONLY to display it back to its owner.
//!
//! Authentication never decrypts; reveal never authenticates.
//!
//! The key carries the MINTING user's identity (`created_by`), and `resolve_api_key` reads the
//! creator's LIVE `users` row for email + role on every request — so a key cannot outlive the
//! account that minted it, and a demotion takes effect on the next request.
//!
//! Routes (tenant-scoped: every handler binds the caller's OWN tenant, so there is no id in the
//! path to spoof across workspaces):
//!   GET    /api/v1/api-keys                 list this tenant's keys (never the plaintext)
//!   POST   /api/v1/api-keys                 mint one — returns the plaintext ONCE
//!   POST   /api/v1/api-keys/:id/reveal      show it again (requires the caller's password)
//!   POST   /api/v1/api-keys/:id/regenerate  one atomic revoke-and-issue
//!   DELETE /api/v1/api-keys/:id             revoke one (soft: sets `revoked_at`)

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::auth::models::Claims;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Longest-lived credential the app has issued: 100 years, so "durable" is literal and
/// revocation is the `revoked_at` column, never the clock.
const API_KEY_TTL_SECS: usize = 100 * 365 * 24 * 3600;

/// The prefix every minted API key carries (`fs_` + 64 hex chars), so a presented bearer
/// announces whether it is a key or a session token before any lookup.
pub fn is_api_key(token: &str) -> bool {
    token
        .strip_prefix("fs_")
        .is_some_and(|rest| rest.len() == 64 && rest.chars().all(|c| c.is_ascii_hexdigit()))
}

/// hex SHA-256 of the full plaintext token (64 chars). The ONLY form ever persisted.
fn hash_key(token: &str) -> String {
    hex::encode(ring::digest::digest(
        &ring::digest::SHA256,
        token.as_bytes(),
    ))
}

/// Resolve a bearer token that is NOT a valid JWT against the `api_keys` table.
///
/// Returns synthesized [`Claims`] for a live key (stamping `last_used_at` best-effort),
/// or `None` when nothing matches. Called from the auth boundary and the `AuthUser` extractor's
/// JWT-failure arm, so a normal session token never pays for the lookup.
pub async fn resolve_api_key(db: &sqlx::PgPool, token: &str) -> Option<Claims> {
    if !is_api_key(token) {
        return None;
    }
    let hash = hash_key(token);
    let row = sqlx::query(
        "SELECT id, tenant_id, created_by FROM api_keys \
         WHERE key_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(db)
    .await
    .ok()??;

    let id: Uuid = row.get("id");
    let tenant_id: Uuid = row.get("tenant_id");
    let created_by: Option<Uuid> = row.get("created_by");

    // Best-effort usage stamp: a bookkeeping write must never deny a valid key.
    let _ = sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
        .bind(id)
        .execute(db)
        .await;

    // The key carries the MINTING user's identity, and its role + email are read from the LIVE
    // row on every request — so a deleted creator refuses (the boundary's users lookup), and a
    // demoted creator's key loses the platform surface immediately.
    let (email, role) = match created_by {
        Some(uid) => sqlx::query_as::<_, (String, String)>(
            "SELECT email, role FROM users WHERE id = $1 AND is_active = true",
        )
        .bind(uid)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| (String::new(), "user".to_string())),
        None => (String::new(), "user".to_string()),
    };

    let now = chrono::Utc::now().timestamp() as usize;
    Some(Claims {
        sub: created_by.map(|u| u.to_string()).unwrap_or_default(),
        tenant_id: tenant_id.to_string(),
        email,
        role,
        exp: now + API_KEY_TTL_SECS,
        iat: now,
        aud: Some("funnelswift-api".to_string()),
        iss: Some("funnelswift".to_string()),
        impersonating: None,
        wsid: None,
        // THE marker: this Claims came from a durable API key, so the boundary and the erasure
        // handler can refuse an agent holding it (2026-11-24, fleet standard §3.4).
        via_api_key: true,
        // Minted by this app from a stored key (kanban t_a14ad547).
        origin: Some("api_key".to_string()),
    })
}

fn tenant_from_auth(auth: &AuthUser) -> Result<Uuid, AppError> {
    Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::Unauthorized("Invalid tenant in token".to_string()))
}

/// Key management is an ACCOUNT-ADMIN surface, on BOTH doors (`/api/v1/api-keys` and the
/// fleet-standard `/api/personal-api-keys`).
///
/// An issued key is the whole workspace's credential: a plain `user` reaching these routes could
/// rotate the owner's key and walk away with the new plaintext, or revoke the credential every
/// integration depends on. The fleet's role vocabulary is `user | company_admin | admin` (plus
/// `owner` in the older apps), so the check is a membership test, not an equality against one
/// spelling — CoreSwift's version of this check is what the fleet standard was written from.
/// May this session manage the workspace's API keys?
///
/// MEASURED 2026-11-25: signup writes `role = "user"` for the person who creates the workspace
/// (`public_signup_handler`) and mints the tenant's `auto-provisioned` key in the same
/// transaction — so the OWNER of a brand-new account was answered
/// `403 Only an account admin may manage this workspace's API keys` the moment the Settings pane
/// read `GET /api/v1/api-keys`. The key existed and could be neither seen, revealed, rotated nor
/// revoked, which also meant the fleet MCP could never be connected by that customer.
///
/// Authority cannot be a role list here for exactly the reason `team_handler::is_workspace_owner`
/// documents (2026-11-10): a brand-new customer's own owner row IS literally `user`. So the
/// decision is: a platform operator, an appointed account admin — or the workspace's CREATOR, the
/// app's own owner predicate.
pub async fn require_key_admin(state: &AppState, auth: &AuthUser) -> AppResult<()> {
    if auth.is_admin || matches!(auth.role.as_str(), "admin" | "company_admin" | "owner") {
        return Ok(());
    }
    if auth.impersonating.is_some() {
        return Ok(());
    }
    let tenant_id = tenant_from_auth(auth)?;
    if crate::handlers::team_handler::is_workspace_owner(state, auth, tenant_id).await? {
        return Ok(());
    }
    Err(AppError::Forbidden(
        "Only the workspace owner or an account admin may manage this workspace's API keys."
            .to_string(),
    ))
}

fn caller_id(auth: &AuthUser) -> Result<Uuid, AppError> {
    Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user in token".to_string()))
}

/// A key that was just minted. `token` is the plaintext: sealed before the row is written, handed
/// to the caller's UI, never logged.
pub struct MintedKey {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub prefix: String,
}

/// The label every account's auto-provisioned key carries (plan §6.5, David 2026-11-25):
/// `MCP — <account name>`, so the key says what it is for in the API Keys screen and on the MCP
/// card. ONE formatter for every account-creating door (both signups, the operator's workspace
/// create/repair, the portfolio mirror), so a new account can never arrive with a key labelled
/// something else.
///
/// The `api_keys.name` column is `text`, but a name longer than the screen can show is still a
/// defect: truncated to 255 CHARS on a boundary the UI can print, the same ceiling
/// `create_api_key` refuses a hand-typed name at.
pub fn auto_provisioned_key_name(account_name: &str) -> String {
    let name = account_name.trim();
    let base = if name.is_empty() {
        "MCP — this account"
    } else {
        "MCP — "
    };
    let budget = 255usize.saturating_sub(base.chars().count());
    let mut out = String::with_capacity(base.len() + name.len().min(budget));
    out.push_str(base);
    out.extend(name.chars().take(budget));
    out
}

/// Mint one key and INSERT it on the caller's connection. The ONE writer for every mint in this
/// app: the signup transaction, `create_api_key` and regenerate.
pub async fn mint_on(
    conn: &mut sqlx::PgConnection,
    tenant_id: Uuid,
    created_by: Option<Uuid>,
    name: &str,
) -> Result<MintedKey, AppError> {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let token = format!("fs_{}", hex::encode(buf));
    let key_prefix: String = token.chars().take(12).collect();
    let key_hash = hash_key(&token);
    // Fail closed: without PROVIDER_KEY_ENC_SECRET this errors rather than storing the key where
    // nobody can ever read it back.
    let sealed = crate::security::provider_key_crypto::seal_in(&mut *conn, &token)
        .await
        .map_err(|e| AppError::Internal(format!("could not seal the API key: {e}")))?;
    let id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO api_keys (id, tenant_id, name, key_prefix, key_hash, key_sealed, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(name)
    .bind(&key_prefix)
    .bind(&key_hash)
    .bind(&sealed)
    .bind(created_by)
    .execute(&mut *conn)
    .await?;

    // Audit HERE, not in the callers: signup, create and regenerate all funnel through mint_on,
    // so this is the one place a mint cannot bypass. Written on the same connection as the insert,
    // so a mint and its audit row commit or roll back together.
    sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) \
         VALUES ($1, $2, $3, 'api_key_created', 'api_key', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(created_by.map(|u| u.to_string()).unwrap_or_default())
    .bind(id.to_string())
    .bind(json!({ "key_prefix": key_prefix.clone(), "name": name, "source": "mint_on" }))
    .execute(&mut *conn)
    .await?;

    Ok(MintedKey {
        id,
        name: name.to_string(),
        token,
        prefix: key_prefix,
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatedApiKey {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    /// The plaintext, returned exactly once at mint time and retrievable again via reveal.
    pub token: String,
}

/// `GET /api/v1/api-keys` — this tenant's keys, plaintext never included.
pub async fn list_api_keys(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    require_key_admin(&state, &auth).await?;
    let tenant_id = tenant_from_auth(&auth)?;

    let rows = sqlx::query(
        "SELECT id, name, key_prefix, created_at, last_used_at, revoked_at, \
                (key_sealed IS NOT NULL) AS recoverable \
         FROM api_keys WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let keys: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<String, _>("name"),
                "key_prefix": r.get::<String, _>("key_prefix"),
                "created_at": r.get::<chrono::NaiveDateTime, _>("created_at"),
                "last_used_at": r.get::<Option<chrono::NaiveDateTime>, _>("last_used_at"),
                "revoked_at": r.get::<Option<chrono::NaiveDateTime>, _>("revoked_at"),
                "recoverable": r.get::<bool, _>("recoverable"),
            })
        })
        .collect();

    Ok(Json(json!({ "api_keys": keys })))
}

/// `POST /api/v1/api-keys` — mint a key; the plaintext is returned ONCE (and retrievable again
/// via reveal, so it is never "shown once and lost").
pub async fn create_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    // §10 (t_3a7f70c1): a credential must not mint another credential. A key that could mint,
    // rotate or reveal keys is a takeover — revoking the leaked key would not end the access.
    if auth.via_api_key {
        return Err(AppError::Forbidden(
            "This needs a console session. An API key cannot create keys — sign in to the \
             console and press Create key."
                .to_string(),
        ));
    }
    require_key_admin(&state, &auth).await?;
    let tenant_id = tenant_from_auth(&auth)?;
    let created_by = caller_id(&auth).ok();

    let name = req
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "API key".to_string());
    crate::security::display_name::check_len(&name, 255, "name")
        .map_err(AppError::UnprocessableEntity)?;

    // One LIVE key per tenant. The partial unique index (`api_keys (tenant_id) WHERE
    // revoked_at IS NULL`) is the database backstop; this check is the polite door — it turns a
    // second create into a 409 that names Regenerate, instead of letting the blind insert collide
    // with the index and surface as a 500. Checked inside the same transaction as the mint.
    let mut tx = state.pool.begin().await?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM api_keys WHERE tenant_id = $1 AND revoked_at IS NULL LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    if existing.is_some() {
        return Err(AppError::Conflict(
            "you already have a key — use Regenerate".to_string(),
        ));
    }

    // 32 bytes of OS entropy, `fs_` prefix, sealed for display — all in `mint_on`, so the admin
    // action and the automatic signup mint can never disagree about the row's shape.
    let minted = mint_on(&mut tx, tenant_id, created_by, &name).await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(json!(CreatedApiKey {
            id: minted.id,
            name: minted.name,
            key_prefix: minted.prefix,
            token: minted.token,
        })),
    ))
}

/// `POST /api/v1/api-keys/:id/reveal` — show the key again (repeatable), one click from a console
/// session, no password re-prompt, and every reveal audited in `activity_log`.
///
/// David's ruling (2026-11-25, plan §6.2/§8 in
/// `docs/fleet-per-account-keys-and-mcp-2026-11-25.md`): **View is one click.** The key is private
/// by default (only its prefix is ever listed) and each reveal is recorded, so a valid console
/// session is enough — the same friction model as Stripe. No password re-prompt.
///
/// SESSION ONLY. `AuthUser`'s extractor also accepts a durable `fs_…` key as a bearer credential, so
/// a key presented here is refused 403: a read-only key must never be able to obtain the account's
/// read+write key. The classifier is free (`AuthUser::via_api_key`), no hash is paid per View.
///
/// A request body is not read at all, so a legacy `{"password":"…"}` body is accepted and ignored
/// and an older console build keeps working. The audit row carries the key PREFIX only.
pub async fn reveal_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if auth.via_api_key {
        return Err(AppError::Forbidden(
            "This needs a console session. An API key cannot view keys — sign in to the console \
             and press View."
                .to_string(),
        ));
    }
    require_key_admin(&state, &auth).await?;
    let tenant_id = tenant_from_auth(&auth)?;

    let row = sqlx::query(
        "SELECT key_prefix, key_sealed FROM api_keys \
         WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let prefix: String = row.get("key_prefix");
    let sealed: String = row.get("key_sealed");
    let caller = caller_id(&auth)?;

    let token = crate::security::provider_key_crypto::open_in(&state.pool, &sealed)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, key = %id, "API key reveal: sealed copy could not be opened");
            AppError::Internal(
                "This key could not be opened by the server's encryption key. Regenerate it."
                    .to_string(),
            )
        })?;
    if token.is_empty() {
        return Err(AppError::Internal(
            "This key could not be opened by the server's encryption key. Regenerate it."
                .to_string(),
        ));
    }

    // Every reveal is audited: who, when, which prefix — the record that answers "who had this
    // credential" after a misuse.
    if let Err(e) = sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) \
         VALUES ($1, $2, $3, 'api_key_revealed', 'api_key', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(caller.to_string())
    .bind(id.to_string())
    .bind(json!({ "key_prefix": prefix.clone() }))
    .execute(&state.pool)
    .await
    {
        tracing::error!(error = %e, key = %id, "API key reveal: audit row could not be written");
    }
    tracing::info!(key = %id, key_prefix = %prefix, user = %caller, "API key REVEALED");

    Ok(Json(
        json!({ "id": id.to_string(), "key_prefix": prefix, "token": token }),
    ))
}

/// `DELETE /api/v1/api-keys/:id` — soft-revoke (sets `revoked_at`).
pub async fn revoke_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    require_key_admin(&state, &auth).await?;
    let tenant_id = tenant_from_auth(&auth)?;

    let res = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() \
         WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "API key not found or already revoked".to_string(),
        ));
    }

    if let Err(e) = sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id) \
         VALUES ($1, $2, $3, 'api_key_revoked', 'api_key', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(auth.user_id.clone())
    .bind(id.to_string())
    .execute(&state.pool)
    .await
    {
        tracing::error!(error = %e, key = %id, "activity_log insert failed for api_key_revoked");
    }

    Ok(Json(json!({ "message": "API key revoked", "id": id })))
}

/// `POST /api/v1/api-keys/:id/regenerate` — one atomic revoke-and-issue. The old key 401s on the
/// very next request because its `revoked_at` is set in the same transaction that inserts the
/// replacement.
pub async fn regenerate_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    // §10 (t_3a7f70c1): rotate returns a NEW secret in the response. A key that can replace the
    // account's key is a takeover — a leaked key could re-mint itself and outlive its revocation.
    if auth.via_api_key {
        return Err(AppError::Forbidden(
            "This needs a console session. An API key cannot rotate keys — sign in to the \
             console and press Regenerate."
                .to_string(),
        ));
    }
    require_key_admin(&state, &auth).await?;
    let tenant_id = tenant_from_auth(&auth)?;
    let created_by = caller_id(&auth).ok();

    let name: Option<String> = sqlx::query_scalar(
        "SELECT name FROM api_keys WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    let name = name.ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    // Retire the old key FIRST, then mint the replacement, in ONE transaction. The order is the
    // contract: the partial unique index (`api_keys (tenant_id) WHERE revoked_at IS NULL`) refuses
    // a second live key, so the retire must land before the insert. A failure at either step rolls
    // both back and leaves the customer with the key they already had.
    let mut tx = state.pool.begin().await?;
    let retired = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() \
         WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    if retired.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "API key not found or already revoked".to_string(),
        ));
    }
    let minted = mint_on(&mut tx, tenant_id, created_by, &name).await?;
    tx.commit().await?;

    tracing::info!(
        new_key = %minted.id,
        old_key = %id,
        key_prefix = %minted.prefix,
        "API key REGENERATED — the previous key no longer authenticates"
    );

    // Minimal audit: old key dead, new key live (best-effort).
    if let Err(e) = sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) \
         VALUES ($1, $2, $3, 'api_key_regenerated', 'api_key', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(auth.user_id.clone())
    .bind(minted.id.to_string())
    .bind(json!({ "old_key": id.to_string(), "key_prefix": minted.prefix.clone() }))
    .execute(&state.pool)
    .await
    {
        tracing::error!(error = %e, key = %minted.id, "activity_log insert failed for api_key_regenerated");
    }

    Ok((
        StatusCode::CREATED,
        Json(json!(CreatedApiKey {
            id: minted.id,
            name: minted.name,
            key_prefix: minted.prefix,
            token: minted.token,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_prefix_identifies_the_key_form() {
        let mut buf = [7u8; 32];
        rand::thread_rng().fill_bytes(&mut buf);
        let token = format!("fs_{}", hex::encode(buf));
        assert!(is_api_key(&token));
        // fs_ + 64 hex chars.
        assert_eq!(token.len(), 67);
        assert_eq!(hash_key(&token).len(), 64);
        // A JWT and a bare prefix are NOT keys.
        assert!(!is_api_key("eyJhbGciOiJIUzI1NiJ9.xxx.yyy"));
        assert!(!is_api_key("fs_"));
        assert!(!is_api_key(""));
        // The prefix kept for display is a slice of the same plaintext.
        assert_eq!(token.chars().take(12).collect::<String>(), &token[..12]);
    }

    #[test]
    fn distinct_tokens_hash_differently_and_the_same_token_stably() {
        assert_eq!(hash_key("fs_deadbeef"), hash_key("fs_deadbeef"));
        assert_ne!(hash_key("fs_deadbeef"), hash_key("fs_deadbeee"));
    }
}
