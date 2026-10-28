//! `branding_handler` — the logo half of per-tenant email branding (kanban t_c06a32eb).
//!
//! Three arms, and they are deliberately asymmetric:
//!
//! * `POST /api/v1/settings/branding/logo` — authenticated, multipart, the caller's OWN tenant.
//! * `DELETE /api/v1/settings/branding/logo` — same, removes the logo.
//! * `GET /api/v1/branding/logo/:tenant_id` — PUBLIC by design (`auth::route_policy::PUBLIC_ROUTES`):
//!   a mail client renders `<img src>` with no credential of any kind, so a logo that needed a token
//!   would simply never appear. The route can only ever return the image one tenant uploaded, keyed
//!   by an unguessable uuid, with the content type sniffed from the bytes at upload time; a tenant
//!   with no logo answers 404.
//!
//! The image handling itself is NOT here — `crate::image_store` is the one accept/store/serve path,
//! shared with the profile picture (kanban t_ff948669) exactly as the card requires.
//!
//! `logo_url` is written HERE and nowhere else. `settings_handler::update_settings` preserves the
//! stored value when the tenant saves name/colour, so a panel echo cannot un-reference a logo.

use axum::{
    extract::{Path, Request, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::branding;
use crate::error::{AppError, AppResult};
use crate::image_store;
use crate::state::AppState;

/// A tenant's own id, from the caller's credential.
fn caller_tenant(auth: &AuthUser) -> AppResult<Uuid> {
    Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::Unauthorized("Invalid tenant".into()))
}

/// The URL the console and the mail both point at. Version-stamped because the bytes behind it
/// change while the path stays the same, and both the browser and any cache key on the URL.
fn logo_url_for(tenant_id: Uuid) -> String {
    format!(
        "/api/v1/branding/logo/{tenant_id}?v={}",
        chrono::Utc::now().timestamp()
    )
}

/// Read the stored document, defaulting every field to empty.
async fn stored_document(state: &AppState, tenant_id: Uuid) -> serde_json::Value {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT value FROM tenant_settings WHERE tenant_id = $1 AND key = $2",
    )
    .bind(tenant_id)
    .bind(branding::SETTINGS_KEY)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| json!({}))
}

/// The stored document as (brand_name, brand_color), for a write that must not lose them.
fn name_and_color(doc: &serde_json::Value) -> (String, String) {
    let s = |k: &str| {
        doc.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    (s("brand_name"), s("brand_color"))
}

/// `POST /api/v1/settings/branding/logo` — store the caller's tenant logo and return its URL.
///
/// Compile-time literal (the gate refuses SQL built at run time) and an explicit
/// `jsonb_typeof` guard: a document that somehow is not an object is replaced rather than merged
/// into, so the write cannot turn into an array concatenation.
const BRANDING_UPSERT_SQL: &str = "\
INSERT INTO tenant_settings (id, tenant_id, key, value)
VALUES (gen_random_uuid(), $1, 'email_branding', $2)
ON CONFLICT (tenant_id, key) DO UPDATE SET
  value = CASE WHEN jsonb_typeof(tenant_settings.value) = 'object'
               THEN tenant_settings.value || EXCLUDED.value
               ELSE EXCLUDED.value END,
  updated_at = NOW()
RETURNING value";

pub async fn upload_logo(
    auth: AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = caller_tenant(&auth)?;

    // The ONE shared image path: a multipart form OR a raw image body, the 2 MB cap, magic-byte sniff.
    let (content_type, bytes) = image_store::read_request_image(&state, &headers, request).await?;

    sqlx::query(
        r#"INSERT INTO tenant_logos (tenant_id, content_type, bytes, updated_at)
           VALUES ($1, $2, $3, NOW())
           ON CONFLICT (tenant_id) DO UPDATE
             SET content_type = EXCLUDED.content_type,
                 bytes = EXCLUDED.bytes,
                 updated_at = NOW()"#,
    )
    .bind(tenant_id)
    .bind(&content_type)
    .bind(&bytes)
    .execute(&state.pool)
    .await?;

    // Write ONLY the logo_url key of the branding document, so the name/colour the tenant saved
    // earlier survive an upload (and vice versa).
    let logo_url = logo_url_for(tenant_id);
    let stored: serde_json::Value = sqlx::query_scalar(BRANDING_UPSERT_SQL)
        .bind(tenant_id)
        .bind(json!({ "logo_url": logo_url }))
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(json!({
        "status": "ok",
        "logo_url": logo_url,
        "content_type": content_type,
        "branding": stored,
    })))
}

/// `DELETE /api/v1/settings/branding/logo` — remove the logo and un-reference it.
pub async fn delete_logo(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = caller_tenant(&auth)?;

    let removed = sqlx::query("DELETE FROM tenant_logos WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&state.pool)
        .await?
        .rows_affected();

    // Keep the name/colour; clear only the reference. Done as a read-modify-write of the one
    // document this route also writes, so both writers stay on the same key.
    let (name, color) = name_and_color(&stored_document(&state, tenant_id).await);
    sqlx::query(
        r#"INSERT INTO tenant_settings (id, tenant_id, key, value)
           VALUES (gen_random_uuid(), $1, 'email_branding', $2)
           ON CONFLICT (tenant_id, key) DO UPDATE SET value = $2, updated_at = NOW()"#,
    )
    .bind(tenant_id)
    .bind(branding::document(&name, &color, ""))
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"status": "ok", "removed": removed})))
}

/// `POST /api/v1/settings/branding/preview` — mail the CALLER a sample of their own branded email.
///
/// WHY (cron 2026-10-23, parity with IncentiveSwift): a customer can set a logo, a display name and
/// a colour on the dashboard's Email branding card, but had no way to see the result in a real
/// inbox. `render_template` already applies that branding to every template send, so this arm sends
/// the app's own `branding_preview` sample through the normal mail path — the operator sees exactly
/// what a customer receives.
///
/// The recipient is ALWAYS the caller's own address (the request has no body), so this can never be
/// used as a relay, and an address that cannot receive mail is told so plainly instead of being
/// shown a false "sent".
pub async fn preview_email(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = caller_tenant(&auth)?;
    let to = auth.email.trim().to_string();
    if to.is_empty() {
        return Err(AppError::BadRequest(
            "This account has no email address to send the preview to".into(),
        ));
    }
    if let Some(domain) = crate::security::probe_addr::harness_domain(&to) {
        return Ok(Json(json!({
            "sent": false,
            "to": to,
            "detail": format!(
                "'{domain}' is a reserved test address, so it cannot receive mail — no preview was sent."
            )
        })));
    }
    let name: String = sqlx::query_scalar("SELECT name FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .filter(|n: &String| !n.trim().is_empty())
        .unwrap_or_else(|| "there".to_string());
    match crate::email::send_branding_preview_email(&state.pool, tenant_id, &to, &name).await {
        Ok(()) => {
            let branded = branding::load(&state.pool, tenant_id).await.is_some();
            Ok(Json(json!({
                "sent": true,
                "to": to,
                "branded": branded,
                "detail": if branded {
                    "Preview sent to your own address, with your logo and brand name at the top."
                } else {
                    "Preview sent to your own address. Nothing is branded yet, so it carries the FunnelSwift header — set a name or a logo, save, and send it again."
                }
            })))
        }
        Err(detail) => Ok(Json(json!({ "sent": false, "to": to, "detail": detail }))),
    }
}

/// `GET /api/v1/settings/branding/preview-body` — the email this account's branding produces,
/// rendered and returned, NOT sent (cron 2026-10-28, parity with MissedCall Respondr).
///
/// The pane's "How it looks in a mail" block used to be a hand-drawn mirror of the renderer, so it
/// could drift from the mail a customer receives. This answers it with the bytes the mail path would
/// emit, so an operator judges the REAL header (logo, display name, colour rule) without waiting for
/// an inbox and without a send. It is also the only arm that can prove an UPLOADED logo reached the
/// rendered body: `send_branding_preview_email` mails a sample but nothing reads the result back.
///
/// Nothing is mailed (`sent:false`), and no address is ever taken from the request — the recipient
/// fields describe the CALLER, so this cannot be used as a relay.
pub async fn preview_body(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = caller_tenant(&auth)?;
    let to = auth.email.trim().to_string();
    let name: String = sqlx::query_scalar("SELECT name FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .filter(|n: &String| !n.trim().is_empty())
        .unwrap_or_else(|| "there".to_string());
    let r = crate::email::render_branding_preview(&state.pool, tenant_id, &name, &to).await;
    Ok(Json(json!({
        "sent": false,
        "to": to,
        "subject": r.subject,
        "html": r.html,
        "text": r.text,
        "branded": r.branded,
        "logo_is_own": r.own_logo
    })))
}

/// `GET /api/v1/branding/logo/:tenant_id` — stream a tenant's logo. No credential (see the module
/// docs); ids are unguessable uuids and no other tenant datum is reachable from here.
pub async fn get_logo(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
) -> AppResult<Response> {
    let tid = Uuid::parse_str(&tenant_id).map_err(|_| AppError::NotFound("No such logo".into()))?;

    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT content_type, bytes FROM tenant_logos WHERE tenant_id = $1")
            .bind(tid)
            .fetch_optional(&state.pool)
            .await?;

    let (content_type, bytes) = row.ok_or_else(|| AppError::NotFound("No such logo".into()))?;
    image_store::image_response(content_type, bytes)
}
