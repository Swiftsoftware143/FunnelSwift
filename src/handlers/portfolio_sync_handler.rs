//! Internal portfolio sync handler — receives broadcasts from CoreSwift CRM.
//! Protected by x-internal-key header, not JWT.

use crate::{
    error::{AppError, AppResult},
    AppState,
};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{extract::State, http::HeaderMap, response::IntoResponse, Json};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Constant-time string comparison — avoids leaking the internal key via early exit.
fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

/// RFC 4648 Base32, UPPERCASE, padding stripped — the exact encoding the fleet's derived-password
/// rule specifies. `base64` (already a dependency) is Base64 only, so this is the one codec that
/// must be written by hand; the unit tests below pin it against known vectors.
fn base32_encode_upper_no_pad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((data.len() * 8).div_ceil(5));
    let mut buffer: u32 = 0;
    let mut bits: usize = 0;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1F) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1F) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

/// The fleet's derived portfolio password — deterministic, never transmitted in the payload, never
/// logged, and identical across every app that holds the same `INTERNAL_SYNC_KEY`.
///
/// ```text
/// raw      = HMAC-SHA256(key = INTERNAL_SYNC_KEY, message = "portfolio:" + portfolio_uuid_string)
/// password = Base32(raw), RFC 4648, UPPERCASE, padding stripped, FIRST 20 CHARACTERS
/// ```
///
/// Because it is a pure function of the sync key and the portfolio id, a re-sync reproduces the
/// same value — so it can never invalidate a credential the user has since changed (the derivation
/// is only applied at creation/backfill, and an existing user's `password_hash` is never touched).
fn derive_password(sync_key: &str, portfolio_id: Uuid) -> String {
    let message = format!("portfolio:{portfolio_id}");
    let mut mac =
        HmacSha256::new_from_slice(sync_key.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(message.as_bytes());
    let raw = mac.finalize().into_bytes();
    let b32 = base32_encode_upper_no_pad(&raw);
    b32.chars().take(20).collect()
}

/// The slug a mirrored `tenants` row must carry: the caller's own slug (the hub's contract is that
/// `body.slug` is what lands in both rows), or the app's derived unique slug when the caller sent
/// none — ONE rule, called by every arm of this door so the arms cannot drift apart.
///
/// The derived shape is the one `public_signup_handler.rs` already uses for a fresh account
/// (`{name lowercased, spaces to dashes}-{8 hex}`); `tenants.slug` is NOT NULL + UNIQUE
/// (`tenants_slug_key`), so the random suffix is what keeps two different callers apart.
fn resolve_tenant_slug(name: &str, slug: &str) -> String {
    if !slug.trim().is_empty() {
        return slug.to_string();
    }
    let base: String = name
        .trim()
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let base = if base.is_empty() {
        "tenant"
    } else {
        base.as_str()
    };
    let suffix: String = Uuid::new_v4().to_string().chars().take(8).collect();
    format!("{base}-{suffix}")
}

/// Mirror the caller's tenant into THIS app's `tenants` table.
///
/// `portfolio_companies.tenant_id` carries `portfolio_companies_tenant_id_fkey`
/// (REFERENCES tenants(id) ON DELETE CASCADE — read live with pg_constraint), so the parent row has
/// to exist HERE before any company row can be written. A caller-supplied id with no local parent
/// made the company INSERT raise 23503 and the caller got an opaque
/// `500 {"error":"Database error"}` that named nothing (kanban t_78060829, measured live on
/// 1ea4fa90). This is the same mirror the fleet uses on the twin receiver (IncentiveSwift
/// `ensure_account`, kanban t_b5784899 / t_95866a2c) and the same one MissedCall Respondr's twin
/// door gained in kanban t_8cbdbf2d.
///
/// Idempotent on the caller's id — the hub's id IS the mirror's id — and a slug that already belongs
/// to another account is refused with a 409 that NAMES it, never silently replaced.
async fn mirror_tenant(
    tx: &mut sqlx::PgConnection,
    tid: Uuid,
    name: &str,
    slug: &str,
    email: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO tenants (id, name, slug, email) \
         VALUES ($1, $2, $3, NULLIF($4, '')) ON CONFLICT (id) DO NOTHING",
    )
    .bind(tid)
    .bind(name)
    .bind(slug)
    .bind(email)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        // The only unique guard this statement can trip which its arbiter does not absorb is
        // `tenants_slug_key`: a PK conflict IS the ON CONFLICT target. Matched on the SQLSTATE,
        // not on a constraint name (write-validation-parity).
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => AppError::Conflict(
            format!("slug '{slug}' is already in use by another account"),
        ),
        other => other.into(),
    })?;

    // A re-sync fills a blank address; it never rewrites one the operator already holds.
    if !email.trim().is_empty() {
        sqlx::query("UPDATE tenants SET email = $2 WHERE id = $1 AND email IS NULL")
            .bind(tid)
            .bind(email)
            .execute(&mut *tx)
            .await?;
    }

    // The fleet's standing rule: an owner/portfolio account sits on the TOP plan. This door is the
    // one that creates portfolio tenants (SwiftImpact, ZaarHub, Giraudy — David's own businesses),
    // and it used to leave them PLANLESS, so `resolved_limit` answered "not configured" (no cap)
    // while every `has_*` flag resolved false — unlimited and crippled at once, and their own
    // Analytics screen answered 402. The top plan is the highest-priced row (live: agency, 79),
    // resolved by a column rather than a slug so a fresh install assigns its own top tier. Only
    // when the tenant has NO active subscription, so a plan the operator assigned is never
    // overwritten.
    sqlx::query(
        "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date)
         SELECT $1, $2, p.id, 'active', NOW() FROM plans p
          WHERE NOT EXISTS (SELECT 1 FROM tenant_plan_subscriptions t
                             WHERE t.tenant_id = $2 AND t.status = 'active')
          ORDER BY p.price DESC, p.slug ASC LIMIT 1",
    )
    .bind(Uuid::new_v4())
    .bind(tid)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The company row both fallback arms write, sharing the resolved slug with the mirror parent.
/// `account_tenant_id` is the COMPANY'S OWN account (kanban t_4a9454c1): this door mirrors that
/// tenant into `tenants` immediately before, so it is the same id as `tenant_id` here. The DO
/// UPDATE arm never clears a link an earlier run established.
const SQL_COMPANY_UPSERT: &str =
    "INSERT INTO portfolio_companies (id, tenant_id, account_tenant_id, name, slug, email) \
     VALUES ($1, $2, $2, $3, $4, $5) \
     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, slug = EXCLUDED.slug, \
     email = EXCLUDED.email, \
     account_tenant_id = COALESCE(portfolio_companies.account_tenant_id, EXCLUDED.account_tenant_id), \
     updated_at = NOW()";

/// What the provisioning half of a sync did, so the caller can see which rows changed and the
/// welcome-mail arm can send exactly once (only when the owner user was actually created).
struct ProvisionOutcome {
    /// The workspace's owner user — either the one just created or the one already present.
    user_id: Uuid,
    /// True only when THIS sync created the user. The welcome mail is gated on this: a workspace
    /// that already had a login is never re-mailed and its password is never touched.
    user_created: bool,
    /// The derived password, meaningful only when `user_created` is true (otherwise empty).
    password: String,
    /// The workspace's live key prefix, whether minted by this sync or already present.
    key_prefix: Option<String>,
    /// True only when THIS sync minted the key.
    key_minted: bool,
}

/// Ensure the workspace holds its owner user and exactly one live API key — the fleet portfolio
/// provisioning standard (2026-11-15) — inside the caller's transaction, idempotently.
///
/// * workspace, no user → create the owner (payload email, role `user` — this app's manual-signup
///   role — Argon2 hash of the DERIVED password) and mint a key; `user_created = true` so the
///   caller sends the welcome mail.
/// * workspace + user, no key → mint the key, `user_created = false` (no re-mail, no password
///   reset).
/// * everything present → no-op; both flags false.
///
/// The role is `user`, NOT a foreign `owner` word: FunnelSwift's signup writes `role = "user"` and
/// resolves workspace ownership by the OLDEST active user (`team_handler::is_workspace_creator`),
/// so a single-user portfolio account is the owner by construction — exactly the shape a manual
/// signup produces and the shape the sibling portfolio tenant (Xride) already carries. The
/// standard's own non-negotiable is "the same role a manual signup would get in that app".
async fn provision_workspace(
    tx: &mut sqlx::PgConnection,
    sync_key: &str,
    tenant_id: Uuid,
    portfolio_id: Uuid,
    name: &str,
    email: &str,
) -> Result<ProvisionOutcome, AppError> {
    // Owner lookup: the workspace's oldest active person, the same predicate
    // `is_workspace_creator` uses. A workspace that already has ANY login keeps it untouched.
    let existing_user: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE tenant_id = $1 AND is_active = true \
         ORDER BY created_at, id LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (user_id, user_created, password) = match existing_user {
        Some(uid) => (uid, false, String::new()),
        None => {
            let derived = derive_password(sync_key, portfolio_id);
            let salt = SaltString::generate(&mut OsRng);
            let hash = Argon2::default()
                .hash_password(derived.as_bytes(), &salt)
                .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
                .to_string();
            let uid = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active) \
                 VALUES ($1, $2, $3, $4, $5, 'user', true)",
            )
            .bind(uid)
            .bind(tenant_id)
            .bind(email)
            .bind(&hash)
            .bind(name)
            .execute(&mut *tx)
            .await?;
            (uid, true, derived)
        }
    };

    // Seat the seven default lead stages when the workspace has none, exactly as a manual signup
    // (`public_signup`) and the app's own repair door (`create_owner_login`) do — a portfolio
    // account must be indistinguishable from a signup account, and the sibling portfolio tenant
    // (Xride) carries these rows. Guarded on NOT-EXISTS so a re-sync adds nothing.
    let stages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenant_settings WHERE tenant_id = $1 AND key = 'lead_stages'",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    if stages == 0 {
        sqlx::query(
            "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(json!([
            "New",
            "Contacted",
            "Qualified",
            "Proposal",
            "Negotiation",
            "Closed Won",
            "Closed Lost"
        ]))
        .execute(&mut *tx)
        .await?;
    }

    // One live key per workspace, enforced by the partial unique index
    // (`idx_api_keys_tenant_live_unique`). Mint only when there is none; a re-sync never mints a
    // second key and never revokes the one the owner already holds.
    let live_prefix: Option<String> = sqlx::query_scalar(
        "SELECT key_prefix FROM api_keys WHERE tenant_id = $1 AND revoked_at IS NULL LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (key_prefix, key_minted) = match live_prefix {
        Some(prefix) => (Some(prefix), false),
        None => {
            // `mint_on` is the ONE key writer: hash + sealed copy + prefix + audit row, all on this
            // connection so they commit or roll back with the account.
            let minted = crate::handlers::api_keys::mint_on(
                &mut *tx,
                tenant_id,
                Some(user_id),
                &crate::handlers::api_keys::auto_provisioned_key_name(name),
            )
            .await?;
            (Some(minted.prefix), true)
        }
    };

    Ok(ProvisionOutcome {
        user_id,
        user_created,
        password,
        key_prefix,
        key_minted,
    })
}

/// Send the welcome mail for a just-provisioned portfolio workspace — best effort, AFTER commit.
///
/// The standard is explicit: a mail failure must never fail the sync (the account already exists),
/// so the failure is logged loudly and reported in the response, never returned as an error. The
/// mail goes to the PORTFOLIO email only, never to the operator.
async fn send_welcome(
    state: &AppState,
    tenant_id: Uuid,
    name: &str,
    email: &str,
    password: &str,
    key_prefix: &str,
) -> bool {
    match crate::email::send_portfolio_provision_email(
        &state.pool,
        tenant_id,
        email,
        name,
        password,
        key_prefix,
    )
    .await
    {
        Ok(()) => {
            tracing::info!(email = %email, tenant = %tenant_id, "portfolio sync: welcome email sent");
            true
        }
        Err(e) => {
            tracing::error!(
                email = %email,
                tenant = %tenant_id,
                error = %e,
                "portfolio sync: workspace provisioned but the WELCOME EMAIL FAILED"
            );
            false
        }
    }
}

/// The workspace this app ALREADY holds for a portfolio company, found by the company's stable
/// identity (slug → name → email) rather than by the caller's id.
///
/// WHY (adopt-before-create, 2026-11-15): the hub changed which id it sends — it used to carry its
/// own ADMIN workspace id for every company, and now carries each company's OWN id
/// (`tenant_id == portfolio_id`, CoreSwift `src/portfolio/sync.rs`). This app already held
/// Giraudy/ZaarHub/SwiftImpact under the OLD ids, so a re-sync whose id matched nothing minted a
/// SECOND, empty workspace beside the real one (cleaned 2026-11-15). Matching on the identity this
/// app already stores finds the workspace that exists and adopts it instead of minting a duplicate.
///
/// The TENANT (workspace) is matched first and in full (slug → name → email), because the workspace
/// is the thing a re-sync must never duplicate. The `portfolio_companies` row is only a FALLBACK
/// source of a workspace id (a company whose tenant this app never mirrored), never a preferred
/// match — a stale company row can point at a shared/admin workspace while the company's own tenant
/// carries the correct name.
struct AdoptedCompany {
    /// The existing workspace (tenants.id) to reuse in place of the caller's id.
    tenant_id: Uuid,
    /// The existing portfolio_companies.id to refresh in place (None when this app holds a
    /// workspace for the company but no company row yet).
    company_id: Option<Uuid>,
}

/// Resolve the `portfolio_companies` row to refresh alongside the adopted workspace: the row already
/// linked to it, else the row matching the company's identity (slug → name → email) — the pre-fix
/// link may point at a shared/admin workspace while the adopted tenant is the company's own.
async fn resolve_company_id(
    db: &sqlx::PgPool,
    tenant_id: Uuid,
    name: &str,
    slug: &str,
    email: &str,
) -> Result<Option<Uuid>, AppError> {
    let by_tenant: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM portfolio_companies WHERE tenant_id = $1 LIMIT 1")
            .bind(tenant_id)
            .fetch_optional(db)
            .await?;
    if by_tenant.is_some() {
        return Ok(by_tenant);
    }
    if !slug.trim().is_empty() {
        let by_slug: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM portfolio_companies WHERE slug = $1 LIMIT 1")
                .bind(slug.trim())
                .fetch_optional(db)
                .await?;
        if by_slug.is_some() {
            return Ok(by_slug);
        }
    }
    if !name.trim().is_empty() {
        let by_name: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM portfolio_companies WHERE name = $1 LIMIT 1")
                .bind(name.trim())
                .fetch_optional(db)
                .await?;
        if by_name.is_some() {
            return Ok(by_name);
        }
    }
    if !email.trim().is_empty() {
        let by_email: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM portfolio_companies WHERE email = $1 LIMIT 1")
                .bind(email.trim())
                .fetch_optional(db)
                .await?;
        if by_email.is_some() {
            return Ok(by_email);
        }
    }
    Ok(None)
}

/// Find the workspace this app already holds for a portfolio company — slug → name → email, in the
/// `tenants` table first (the workspace itself) and then, as a fallback, the `portfolio_companies`
/// row's parent. Returns None only when this app holds nothing for the company, which is exactly the
/// case the create arm may mint a fresh workspace for.
async fn find_existing_company(
    db: &sqlx::PgPool,
    name: &str,
    slug: &str,
    email: &str,
) -> Result<Option<AdoptedCompany>, AppError> {
    // 1. The workspace (tenants), slug → name → email.
    if !slug.trim().is_empty() {
        let t: Option<Uuid> = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = $1 LIMIT 1")
            .bind(slug.trim())
            .fetch_optional(db)
            .await?;
        if let Some(t) = t {
            return Ok(Some(AdoptedCompany {
                tenant_id: t,
                company_id: resolve_company_id(db, t, name, slug, email).await?,
            }));
        }
    }
    if !name.trim().is_empty() {
        let t: Option<Uuid> = sqlx::query_scalar("SELECT id FROM tenants WHERE name = $1 LIMIT 1")
            .bind(name.trim())
            .fetch_optional(db)
            .await?;
        if let Some(t) = t {
            return Ok(Some(AdoptedCompany {
                tenant_id: t,
                company_id: resolve_company_id(db, t, name, slug, email).await?,
            }));
        }
    }
    if !email.trim().is_empty() {
        let t: Option<Uuid> = sqlx::query_scalar("SELECT id FROM tenants WHERE email = $1 LIMIT 1")
            .bind(email.trim())
            .fetch_optional(db)
            .await?;
        if let Some(t) = t {
            return Ok(Some(AdoptedCompany {
                tenant_id: t,
                company_id: resolve_company_id(db, t, name, slug, email).await?,
            }));
        }
    }

    // 2. Fallback — a company whose tenant this app never mirrored: its parent id is the workspace
    //    to adopt. slug → name → email.
    if !slug.trim().is_empty() {
        let t: Option<Uuid> =
            sqlx::query_scalar("SELECT tenant_id FROM portfolio_companies WHERE slug = $1 LIMIT 1")
                .bind(slug.trim())
                .fetch_optional(db)
                .await?;
        if let Some(t) = t {
            return Ok(Some(AdoptedCompany {
                tenant_id: t,
                company_id: resolve_company_id(db, t, name, slug, email).await?,
            }));
        }
    }
    if !name.trim().is_empty() {
        let t: Option<Uuid> =
            sqlx::query_scalar("SELECT tenant_id FROM portfolio_companies WHERE name = $1 LIMIT 1")
                .bind(name.trim())
                .fetch_optional(db)
                .await?;
        if let Some(t) = t {
            return Ok(Some(AdoptedCompany {
                tenant_id: t,
                company_id: resolve_company_id(db, t, name, slug, email).await?,
            }));
        }
    }
    if !email.trim().is_empty() {
        let t: Option<Uuid> = sqlx::query_scalar(
            "SELECT tenant_id FROM portfolio_companies WHERE email = $1 LIMIT 1",
        )
        .bind(email.trim())
        .fetch_optional(db)
        .await?;
        if let Some(t) = t {
            return Ok(Some(AdoptedCompany {
                tenant_id: t,
                company_id: resolve_company_id(db, t, name, slug, email).await?,
            }));
        }
    }

    Ok(None)
}

/// Adopt an existing workspace for a portfolio re-sync: refresh its name/slug/email and its company
/// row (re-pointing the company to the adopted workspace), then run the SAME provisioning the create
/// arm runs (owner user if missing, one live key if missing, welcome mail only when the user was
/// actually created). Never mints a second tenant.
async fn adopt_existing_company(
    state: &AppState,
    found: AdoptedCompany,
    payload_pid: Option<Uuid>,
    name: &str,
    slug: &str,
    email: &str,
    can_provision: bool,
) -> AppResult<Json<Value>> {
    let tenant_slug = resolve_tenant_slug(name, slug);
    let mut tx = state.db.begin().await?;

    // 1. Refresh the existing workspace's name + slug. Email is filled by `mirror_tenant` below (a
    //    re-sync fills a blank address, never rewrites one the operator already holds). A slug that
    //    belongs to another account is refused with a 409 that NAMES it, the same refusal the mirror
    //    gives on INSERT.
    sqlx::query("UPDATE tenants SET name = $1, slug = $2, updated_at = NOW() WHERE id = $3")
        .bind(name)
        .bind(&tenant_slug)
        .bind(found.tenant_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
                AppError::Conflict(format!(
                    "slug '{tenant_slug}' is already in use by another account"
                ))
            }
            other => other.into(),
        })?;

    // 2. Fill the workspace email if blank + seat the top plan — the create arm's `mirror_tenant`
    //    does exactly this; its INSERT is an `ON CONFLICT (id) DO NOTHING` no-op here.
    mirror_tenant(&mut tx, found.tenant_id, name, &tenant_slug, email).await?;

    // 3. Refresh + re-point the company row. The pre-fix link may point at a shared/admin workspace;
    //    the company's OWN tenant is the correct scoping parent. `account_tenant_id` is filled by the
    //    same resolved tenant and never cleared (kanban t_4a9454c1).
    match (found.company_id, payload_pid) {
        (Some(cid), _) => {
            sqlx::query(
                "UPDATE portfolio_companies SET name = $1, slug = $2, email = $3, tenant_id = $4, \
                 account_tenant_id = COALESCE(account_tenant_id, $4), updated_at = NOW() WHERE id = $5",
            )
            .bind(name)
            .bind(&tenant_slug)
            .bind(email)
            .bind(found.tenant_id)
            .bind(cid)
            .execute(&mut *tx)
            .await?;
        }
        (None, Some(pid)) => {
            sqlx::query(SQL_COMPANY_UPSERT)
                .bind(pid)
                .bind(found.tenant_id)
                .bind(name)
                .bind(&tenant_slug)
                .bind(email)
                .execute(&mut *tx)
                .await?;
        }
        (None, None) => {}
    }

    // 4. Provision — owner user if missing, one live key if missing, welcome mail only if the user
    //    was actually created (the fleet standard, same as the create arm).
    let prov = if can_provision {
        match payload_pid {
            Some(pid) => {
                provision_workspace(
                    &mut tx,
                    &state.internal_sync_key,
                    found.tenant_id,
                    pid,
                    name,
                    email,
                )
                .await?
            }
            None => ProvisionOutcome {
                user_id: Uuid::nil(),
                user_created: false,
                password: String::new(),
                key_prefix: None,
                key_minted: false,
            },
        }
    } else {
        ProvisionOutcome {
            user_id: Uuid::nil(),
            user_created: false,
            password: String::new(),
            key_prefix: None,
            key_minted: false,
        }
    };

    tx.commit().await?;

    let welcome_sent = if prov.user_created {
        send_welcome(
            state,
            found.tenant_id,
            name,
            email,
            &prov.password,
            prov.key_prefix.as_deref().unwrap_or(""),
        )
        .await
    } else {
        false
    };

    Ok(Json(json!({
        "status": if prov.user_created || prov.key_minted { "synced" } else { "already_provisioned" },
        "adopted": true,
        "tenant_id": found.tenant_id.to_string(),
        "portfolio_id": found.company_id
            .map(|c| c.to_string())
            .or_else(|| payload_pid.map(|p| p.to_string())),
        "user_created": prov.user_created,
        "key_minted": prov.key_minted,
        "welcome_sent": welcome_sent,
        "key_prefix": prov.key_prefix,
    })))
}

/// POST /api/v1/internal/portfolio-sync
/// Accepts x-internal-key header for authentication.
pub async fn portfolio_sync_internal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<impl IntoResponse> {
    let key = headers
        .get("x-internal-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if state.internal_sync_key.is_empty() || !ct_eq(key, &state.internal_sync_key) {
        return Err(AppError::Unauthorized("Invalid internal key".into()));
    }

    let action = body
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("create");
    let portfolio_id = body
        .get("portfolio_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());
    let tenant_id = body
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let slug = body
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let email = body
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // The sync door writes this value into BOTH `portfolio_companies.email` and the mirrored
    // `tenants.email`, so a malformed one becomes a permanently unreachable contact on this app —
    // the same rule the panel door carries (`select_portfolio_company` neighbours in
    // `portfolio_handler.rs`). Absent stays legal. Precedent: ADASwift `portfolio_handler.rs`
    // (kanban t_5230513b), fleet-wide lead guard 2026-10-29.
    crate::security::email_addr::check_lead_address(&email).map_err(AppError::Validation)?;

    // Provisioning needs an address (users.email is NOT NULL); a payload without one stays a legal
    // mirror — the workspace is just left as the contact-less shell it always was, and the missing
    // owner is filled by a later sync that carries the address.
    let can_provision = !email.trim().is_empty();

    // ── adopt-before-create (2026-11-15) ────────────────────────────────────────────────────────
    // A re-sync of a company this app ALREADY holds must adopt the existing workspace, never mint a
    // duplicate beside it. The hub changed which id it sends (the company's OWN id instead of its
    // admin-workspace id), so the caller's id no longer matches the id this app stored; the match is
    // the company's stable identity — slug → name → email. Only when NOTHING matches does the
    // create/update arm below mint a fresh workspace.
    if action == "create" || action == "update" {
        if let Some(found) = find_existing_company(&state.db, &name, &slug, &email).await? {
            return adopt_existing_company(
                &state,
                found,
                portfolio_id,
                &name,
                &slug,
                &email,
                can_provision,
            )
            .await;
        }
    }

    match action {
        "create" => {
            if let (Some(pid), Some(tid)) = (portfolio_id, tenant_id) {
                let tenant_slug = resolve_tenant_slug(&name, &slug);
                let mut tx = state.db.begin().await?;
                mirror_tenant(&mut tx, tid, &name, &tenant_slug, &email).await?;
                sqlx::query(SQL_COMPANY_UPSERT)
                    .bind(pid)
                    .bind(tid)
                    .bind(&name)
                    .bind(&tenant_slug)
                    .bind(&email)
                    .execute(&mut *tx)
                    .await?;
                let prov = if can_provision {
                    provision_workspace(&mut tx, &state.internal_sync_key, tid, pid, &name, &email)
                        .await?
                } else {
                    ProvisionOutcome {
                        user_id: Uuid::nil(),
                        user_created: false,
                        password: String::new(),
                        key_prefix: None,
                        key_minted: false,
                    }
                };
                tx.commit().await?;
                let welcome_sent = if prov.user_created {
                    send_welcome(
                        &state,
                        tid,
                        &name,
                        &email,
                        &prov.password,
                        prov.key_prefix.as_deref().unwrap_or(""),
                    )
                    .await
                } else {
                    false
                };
                return Ok(Json(json!({
                    "status": if prov.user_created || prov.key_minted { "synced" } else { "already_provisioned" },
                    "user_created": prov.user_created,
                    "key_minted": prov.key_minted,
                    "welcome_sent": welcome_sent,
                    "key_prefix": prov.key_prefix,
                })));
            }
        }
        "update" => {
            if let Some(pid) = portfolio_id {
                let rows = sqlx::query("UPDATE portfolio_companies SET name = $1, slug = $2, email = $3, updated_at = NOW() WHERE id = $4")
                    .bind(&name).bind(&slug).bind(&email).bind(pid)
                    .execute(&state.db).await?;
                if rows.rows_affected() == 0 {
                    // Nothing matched, so this app holds no company row for `pid`: this is the arm
                    // that catches up after a `create` broadcast this app MISSED (the hub's
                    // broadcast is per-app, best-effort and only warns on a non-2xx — CoreSwift
                    // src/portfolio/sync.rs). The old fallback bound the CALLER's `tenant_id`
                    // straight into `portfolio_companies`, whose `portfolio_companies_tenant_id_fkey`
                    // needs a `tenants` row HERE, so an unknown caller tenant raised 23503 and the
                    // caller got an opaque `500 {"error":"Database error"}` naming nothing (kanban
                    // t_78060829, measured live on 1ea4fa90 @ 2026-10-08T18:10:41Z). It now mirrors
                    // the caller's tenant exactly as the create arm of this same door does, and
                    // refuses with a 400 that NAMES what is missing when it cannot mirror — never a
                    // 500, never a silent 200 that wrote nothing.
                    let Some(tid) = tenant_id else {
                        return Err(AppError::BadRequest(
                            "update for a portfolio company this app has no row for requires 'tenant_id' to mirror as its parent".into(),
                        ));
                    };
                    let tenant_slug = resolve_tenant_slug(&name, &slug);
                    let mut tx = state.db.begin().await?;
                    mirror_tenant(&mut tx, tid, &name, &tenant_slug, &email).await?;
                    sqlx::query(SQL_COMPANY_UPSERT)
                        .bind(pid)
                        .bind(tid)
                        .bind(&name)
                        .bind(&tenant_slug)
                        .bind(&email)
                        .execute(&mut *tx)
                        .await?;
                    let prov = if can_provision {
                        provision_workspace(
                            &mut tx,
                            &state.internal_sync_key,
                            tid,
                            pid,
                            &name,
                            &email,
                        )
                        .await?
                    } else {
                        ProvisionOutcome {
                            user_id: Uuid::nil(),
                            user_created: false,
                            password: String::new(),
                            key_prefix: None,
                            key_minted: false,
                        }
                    };
                    tx.commit().await?;
                    let welcome_sent = if prov.user_created {
                        send_welcome(
                            &state,
                            tid,
                            &name,
                            &email,
                            &prov.password,
                            prov.key_prefix.as_deref().unwrap_or(""),
                        )
                        .await
                    } else {
                        false
                    };
                    return Ok(Json(json!({
                        "status": if prov.user_created || prov.key_minted { "synced" } else { "already_provisioned" },
                        "user_created": prov.user_created,
                        "key_minted": prov.key_minted,
                        "welcome_sent": welcome_sent,
                        "key_prefix": prov.key_prefix,
                    })));
                }
                // The company row already existed: this is the "update when provisioning is
                // missing" arm. Provision against the company's OWN tenant (the row's scoping key,
                // never a caller-supplied guess), and only the rows that are actually absent.
                let existing_tid: Option<Uuid> =
                    sqlx::query_scalar("SELECT tenant_id FROM portfolio_companies WHERE id = $1")
                        .bind(pid)
                        .fetch_optional(&state.db)
                        .await?;
                let Some(tid) = existing_tid else {
                    // Unreachable in practice (the UPDATE above matched a row), but never a 500.
                    return Ok(Json(json!({"status": "synced"})));
                };
                let mut tx = state.db.begin().await?;
                let prov = if can_provision {
                    provision_workspace(&mut tx, &state.internal_sync_key, tid, pid, &name, &email)
                        .await?
                } else {
                    ProvisionOutcome {
                        user_id: Uuid::nil(),
                        user_created: false,
                        password: String::new(),
                        key_prefix: None,
                        key_minted: false,
                    }
                };
                tx.commit().await?;
                let welcome_sent = if prov.user_created {
                    send_welcome(
                        &state,
                        tid,
                        &name,
                        &email,
                        &prov.password,
                        prov.key_prefix.as_deref().unwrap_or(""),
                    )
                    .await
                } else {
                    false
                };
                return Ok(Json(json!({
                    "status": if prov.user_created || prov.key_minted { "synced" } else { "already_provisioned" },
                    "user_created": prov.user_created,
                    "key_minted": prov.key_minted,
                    "welcome_sent": welcome_sent,
                    "key_prefix": prov.key_prefix,
                })));
            }
        }
        "delete" => {
            if let Some(pid) = portfolio_id {
                sqlx::query("DELETE FROM portfolio_companies WHERE id = $1")
                    .bind(pid)
                    .execute(&state.db)
                    .await?;
            }
        }
        _ => return Err(AppError::BadRequest("Invalid action".into())),
    }

    Ok(Json(json!({"status": "synced"})))
}

#[cfg(test)]
mod tests {
    use super::{base32_encode_upper_no_pad, derive_password};

    /// The derivation is a pure function of the sync key and the portfolio id: deterministic,
    /// 20 chars, and Base32-uppercase. The fleet's own verification token — BV Premiere's derived
    /// password hashes to `sha256` beginning `a1647a97a644` — is asserted against the LIVE key in
    /// the acceptance probe, not here (the sync key is a secret and must not be embedded in tests).
    #[test]
    fn derived_password_is_deterministic_uppercase_base32_of_20_chars() {
        let sync_key = "0123456789abcdef0123456789abcdef0123456789abcdef"; // length matches live key
        let portfolio_id = uuid::Uuid::nil();
        let password = derive_password(sync_key, portfolio_id);
        assert_eq!(password.len(), 20);
        assert!(
            password
                .chars()
                .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)),
            "Base32-uppercase alphabet only: {password}"
        );
        // Round-trip sanity: the value is deterministic.
        assert_eq!(password, derive_password(sync_key, portfolio_id));
        // A different portfolio id yields a different password.
        let other = derive_password(sync_key, uuid::Uuid::new_v4());
        assert_ne!(password, other);
    }

    /// RFC 4648 Base32, uppercase, no padding — pinned against the standard's RFC 4648 test vector
    /// (`foobar` → `MZXW6YTBOI`, `foo` → `MZXW6===` padded).
    #[test]
    fn base32_is_rfc_4648_uppercase_no_pad() {
        assert_eq!(base32_encode_upper_no_pad(b"f"), "MY");
        assert_eq!(base32_encode_upper_no_pad(b"fo"), "MZXQ");
        assert_eq!(base32_encode_upper_no_pad(b"foo"), "MZXW6");
        assert_eq!(base32_encode_upper_no_pad(b"foob"), "MZXW6YQ");
        assert_eq!(base32_encode_upper_no_pad(b"foobar"), "MZXW6YTBOI");
    }
}
