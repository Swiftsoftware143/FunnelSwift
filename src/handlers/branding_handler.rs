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
    extract::{Multipart, Path, State},
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
    mut multipart: Multipart,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = caller_tenant(&auth)?;

    // The ONE shared image path: multipart read, 2 MB cap, magic-byte sniff.
    let (content_type, bytes) = image_store::read_uploaded_image(&mut multipart).await?;

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
