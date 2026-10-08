//! Internal portfolio sync handler — receives broadcasts from CoreSwift CRM.
//! Protected by x-internal-key header, not JWT.

use crate::{
    error::{AppError, AppResult},
    AppState,
};
use axum::{extract::State, http::HeaderMap, response::IntoResponse, Json};
use serde_json::{json, Value};
use uuid::Uuid;

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
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING",
    )
    .bind(tid)
    .bind(name)
    .bind(slug)
    .execute(tx)
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
    Ok(())
}

/// The company row both fallback arms write, sharing the resolved slug with the mirror parent.
const SQL_COMPANY_UPSERT: &str =
    "INSERT INTO portfolio_companies (id, tenant_id, name, slug, email) \
     VALUES ($1, $2, $3, $4, $5) \
     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, slug = EXCLUDED.slug, \
     email = EXCLUDED.email, updated_at = NOW()";

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

    match action {
        "create" => {
            if let (Some(pid), Some(tid)) = (portfolio_id, tenant_id) {
                // `tenants.slug` carries UNIQUE `tenants_slug_key`, and the slug on this arm is
                // CALLER-SUPPLIED (`body.slug`), so a slug that already belongs to any tenant made
                // the tenants INSERT raise 23505 — and `.ok()` DISCARDED it. The next statement then
                // wrote a `portfolio_companies` row whose `tenant_id` had no parent, so the FK
                // (`portfolio_companies_tenant_id_fkey`) raised and the caller got an opaque
                // `500 {"error":"Database error"}` naming nothing (measured live on 1ea4fa90,
                // 2026-10-08T18:10:41Z — the same run that measured it on the update arm below).
                // The two statements now run in ONE transaction and the error is PROPAGATED: a
                // caller-supplied slug is honoured when it is actually free and refused with a 409
                // that names it when it is taken — never silently replaced, because the hub's
                // contract is that `body.slug` is what lands in both rows. A create that supplies NO
                // slug is derived by the app's own rule (one rule, both arms — this door cannot
                // drift from itself).
                let tenant_slug = resolve_tenant_slug(&name, &slug);
                let mut tx = state.db.begin().await?;
                mirror_tenant(&mut tx, tid, &name, &tenant_slug).await?;
                sqlx::query(SQL_COMPANY_UPSERT)
                    .bind(pid)
                    .bind(tid)
                    .bind(&name)
                    .bind(&tenant_slug)
                    .bind(&email)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
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
                    // The UPDATE above matched no row, so there is nothing to roll back; the
                    // transaction covers the two rows that must land together (mirror parent +
                    // company), so a refusal leaves the DB untouched.
                    let mut tx = state.db.begin().await?;
                    mirror_tenant(&mut tx, tid, &name, &tenant_slug).await?;
                    sqlx::query(SQL_COMPANY_UPSERT)
                        .bind(pid)
                        .bind(tid)
                        .bind(&name)
                        .bind(&tenant_slug)
                        .bind(&email)
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                }
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
