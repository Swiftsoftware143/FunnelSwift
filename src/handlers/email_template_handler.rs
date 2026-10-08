//! Email Template Admin API
//! Manage email templates (welcome, password_reset, purchase_confirmed, etc.)

use axum::{extract::State, http::StatusCode, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// The one string a masked secret is replaced with in responses — the SAME marker the tenant
/// settings view answers with (`email_provider::SECRET_MASK`), so the two credential surfaces
/// cannot drift apart.
const MASK: &str = crate::email_provider::SECRET_MASK;

/// True when an incoming value is really the mask, or blank ("leave the stored credential alone").
/// Shared with the tenant settings write path so both surfaces agree what "masked" means.
fn is_masked(v: &str) -> bool {
    crate::email_provider::is_masked(v)
}

/// True when a body — AFTER the masked-secret restore and the `<field>_set` strip — names no key
/// at all, i.e. there is nothing to write (kanban t_e14b4c36). Kept as a named predicate so the
/// "empty document" arm is unit-tested without a database.
fn email_config_body_is_empty(body: &serde_json::Value) -> bool {
    body.as_object().map(|o| o.is_empty()).unwrap_or(true)
}

/// The write arm for the global email config (kanban t_e14b4c36).
///
/// The row is ONE jsonb document (`admin_settings.value`), but this admin surface has no contract
/// that a caller sends every key: a POST of `{}` used to REPLACE the whole document and silently
/// drop `api_url` / `provider` / `from_address` (measured live 2026-10-02, restored from the daily
/// dump). MERGE instead of replace — `stored || incoming`, the jsonb shallow merge in which a key
/// named by the caller wins — so a partial document touches only the keys it names.
/// `jsonb_typeof` keeps a legacy non-object value from turning the merge into an array
/// concatenation; the column is `NOT NULL DEFAULT '{}'` so the guard is belt-and-braces.
///
/// Compile-time literal (gate rule 5d refuses SQL built at run time) and pinned by the unit test
/// below, so a silent revert to `value = $1::jsonb` cannot pass.
const EMAIL_CONFIG_UPSERT_SQL: &str = "\
INSERT INTO admin_settings (key, value, description, updated_at)
VALUES ('email', $1::jsonb, 'Global system email provider (admin-editable)', NOW())
ON CONFLICT (key) DO UPDATE SET
  value = CASE WHEN jsonb_typeof(admin_settings.value) = 'object'
               THEN admin_settings.value || EXCLUDED.value
               ELSE EXCLUDED.value END,
  updated_at = NOW()
RETURNING value";

/// Defence in depth for the platform-wide e-mail surface (kanban t_9cf378bc): every handler in
/// this module is mounted only under `/api/v1/admin/*`, which the router choke point
/// (`auth::global_auth::require_auth`) already refuses for a non-platform-admin token. The
/// check is repeated here so a handler is not authorised merely by being mounted behind that
/// middleware — the same shape ADASwift used in t_d2df1ae1.
fn require_platform_admin(user: &crate::auth::middleware::AuthUser) -> Result<(), AppError> {
    if crate::auth::global_auth::is_platform_admin(&user.role) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Platform admin role required".to_string(),
        ))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailTemplate {
    pub id: Uuid,
    pub template_type: String,
    pub name: String,
    pub subject: String,
    pub body: Option<String>,
    pub html_body: Option<String>,
    pub is_default: bool,
    pub aid: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub template_type: String,
    pub name: String,
    pub subject: String,
    pub body: Option<String>,
    pub html_body: Option<String>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    /// Editable since kanban t_052b4c7a: the panel's Type select now sends `template_type`, and an
    /// update that silently ignored it would let an admin pick a type and see nothing change.
    pub template_type: Option<String>,
    pub name: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub html_body: Option<String>,
    pub is_default: Option<bool>,
}

/// `idx_email_templates_unique` is a PARTIAL unique index: one `is_default = true` row per
/// `template_type` among the rows with `aid IS NULL`. So `is_default = true` while the type already
/// has a default is a 23505 (500) unless the incumbent is demoted first — do it in the SAME
/// transaction as the write, which is what "one default per type" means for the admin panel's
/// toggle. `keep` excludes the row being updated from its own demotion.
async fn demote_default(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    template_type: &str,
    keep: Option<Uuid>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_templates SET is_default = false, updated_at = NOW() \
         WHERE template_type = $1 AND aid IS NULL AND is_default = true AND ($2::uuid IS NULL OR id <> $2)",
    )
    .bind(template_type)
    .bind(keep)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to re-point the default template: {e}")))?;
    Ok(())
}

/// GET /api/v1/admin/email-templates
pub async fn list_templates(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailTemplate>>, AppError> {
    require_platform_admin(&user)?;
    let rows = sqlx::query(
        "SELECT id, template_type, name, subject, body, html_body, is_default, aid, created_at, updated_at FROM email_templates ORDER BY is_default DESC, template_type"
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to fetch templates: {e}")))?;

    let templates = rows
        .iter()
        .map(|r| EmailTemplate {
            id: r.get("id"),
            template_type: r.get("template_type"),
            name: r.get("name"),
            subject: r.get("subject"),
            body: r.get("body"),
            html_body: r.get("html_body"),
            is_default: r.get("is_default"),
            aid: r.get("aid"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();

    Ok(Json(templates))
}

/// POST /api/v1/admin/email-templates
pub async fn create_template(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTemplateRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    require_platform_admin(&user)?;
    let id = Uuid::new_v4();
    let is_default = req.is_default.unwrap_or(false);

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create template: {e}")))?;

    if is_default {
        demote_default(&mut tx, &req.template_type, None).await?;
    }

    sqlx::query(
        "INSERT INTO email_templates (id, template_type, name, subject, body, html_body, is_default) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(id)
    .bind(&req.template_type)
    .bind(&req.name)
    .bind(&req.subject)
    .bind(&req.body)
    .bind(&req.html_body)
    .bind(is_default)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to create template: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create template: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id, "status": "created"})),
    ))
}

/// PUT /api/v1/admin/email-templates/:id
pub async fn update_template(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<UpdateTemplateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_platform_admin(&user)?;
    let existing = sqlx::query(
        "SELECT template_type, name, subject, body, html_body, is_default FROM email_templates WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
    .ok_or_else(|| AppError::NotFound("Template not found".into()))?;

    let template_type: String = req
        .template_type
        .unwrap_or_else(|| existing.get("template_type"));
    let name = req.name.unwrap_or_else(|| existing.get("name"));
    let subject = req.subject.unwrap_or_else(|| existing.get("subject"));
    let body: Option<String> = if req.body.is_some() {
        req.body
    } else {
        existing.get("body")
    };
    let html_body: Option<String> = if req.html_body.is_some() {
        req.html_body
    } else {
        existing.get("html_body")
    };
    let is_default: bool = req.is_default.unwrap_or_else(|| existing.get("is_default"));

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update template: {e}")))?;

    if is_default {
        demote_default(&mut tx, &template_type, Some(id)).await?;
    }

    sqlx::query("UPDATE email_templates SET template_type=$1, name=$2, subject=$3, body=$4, html_body=$5, is_default=$6, updated_at=NOW() WHERE id=$7")
        .bind(&template_type)
        .bind(&name)
        .bind(&subject)
        .bind(&body)
        .bind(&html_body)
        .bind(is_default)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update template: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update template: {e}")))?;

    Ok(Json(serde_json::json!({"id": id, "status": "updated"})))
}

/// DELETE /api/v1/admin/email-templates/:id
pub async fn delete_template(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_platform_admin(&user)?;
    sqlx::query("DELETE FROM email_templates WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to delete template: {e}")))?;

    Ok(Json(serde_json::json!({"id": id, "status": "deleted"})))
}

/// GET /api/v1/admin/email-templates/:id
pub async fn get_template(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<EmailTemplate>, AppError> {
    require_platform_admin(&user)?;
    let r = sqlx::query(
        "SELECT id, template_type, name, subject, body, html_body, is_default, aid, created_at, updated_at FROM email_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
    .ok_or_else(|| AppError::NotFound("Template not found".into()))?;

    Ok(Json(EmailTemplate {
        id: r.get("id"),
        template_type: r.get("template_type"),
        name: r.get("name"),
        subject: r.get("subject"),
        body: r.get("body"),
        html_body: r.get("html_body"),
        is_default: r.get("is_default"),
        aid: r.get("aid"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }))
}

/// GET /api/v1/admin/email-templates/types
pub async fn list_template_types(
    user: crate::auth::middleware::AuthUser,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    require_platform_admin(&user)?;
    Ok(Json(vec![
        serde_json::json!({"type": "welcome", "description": "Sent after user registration", "merge_fields": ["name", "email", "login_url", "app_name"]}),
        serde_json::json!({"type": "password_reset", "description": "Sent when user requests password reset", "merge_fields": ["name", "token", "app_name"]}),
        serde_json::json!({"type": "purchase_confirmed", "description": "Sent after successful payment", "merge_fields": ["name", "plan_name", "login_url", "app_name"]}),
    ]))
}

/// GET /api/v1/admin/email-config — global (system mail) provider config, secrets masked.
/// DB-backed: nothing here reads the process environment.
///
/// Admin-gated in-handler as well as at the router choke point (kanban t_9cf378bc): the
/// handler must not be reachable merely because it is mounted behind the auth middleware.
pub async fn get_email_config(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_platform_admin(&user)?;
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM admin_settings WHERE key = 'email'")
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);

    let cfg = value
        .as_ref()
        .map(|v| crate::email_provider::EmailConfig::from_json(v, "smtp"));

    let mut out = value.unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = out.as_object_mut() {
        // Mask EVERY credential field the crypto vocabulary seals (kanban t_b040a78e item 1). The
        // list used to be two hardcoded names, so a row carrying the legacy SMTP-password aliases
        // (`password` / `pass`, which `EmailConfig::from_json` reads) shipped its CIPHERTEXT to the
        // panel — the seal vocabulary and the reader/masker vocabulary have to be the same one.
        // A field the row does not carry is left alone rather than answered with a marker for a
        // field that is not there, so this surface stays the shape the stored config has.
        //
        // `set` is read from the STORED field, not from an opened value: this route deliberately
        // does not decrypt (the mask is a fixed string, so nothing leaks either way) and reporting
        // a row this deployment cannot open as "not set" would let a subsequent save overwrite the
        // credential with an empty string. The tenant surface, which does open, reports it unset.
        for field in crate::email_provider::CONFIG_SECRET_FIELDS {
            if !obj.contains_key(field) {
                continue;
            }
            let set = obj
                .get(field)
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            obj.insert(
                field.to_string(),
                serde_json::json!(if set { MASK } else { "" }),
            );
            obj.insert(format!("{}_set", field), serde_json::json!(set));
        }
    }

    Ok(Json(serde_json::json!({
        "config": out,
        "configured": cfg.as_ref().map(|c| c.is_configured()).unwrap_or(false),
        "provider": cfg.as_ref().map(|c| c.provider.clone()).unwrap_or_default(),
        "providers": crate::email_provider::available(),
    })))
}

/// POST /api/v1/admin/email-config — save the global system-mail provider.
/// A masked secret coming back from the UI never overwrites the stored one.
pub async fn update_email_config(
    user: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_platform_admin(&user)?;
    let existing: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM admin_settings WHERE key = 'email'")
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let existing = existing.unwrap_or_else(|| serde_json::json!({}));

    let obj = body
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("Expected a JSON object".to_string()))?;

    // The SAME vocabulary seals, masks and restores (kanban t_b040a78e item 1): a masked field
    // coming back from the panel keeps the stored credential whatever it is named — including the
    // legacy `password` / `pass` aliases — and the `<field>_set` markers that `get_email_config`
    // answers with are UI scaffolding, not config, so they are stripped before the row is stored.
    //
    // Only a field the caller actually NAMED is restored (kanban t_e14b4c36). Now that the write is
    // a merge, a field the caller did not name is preserved by the database anyway; restoring it
    // into the body was what made a `{}` document look non-empty and slip past the "nothing to
    // update" refusal below — the exact body that used to delete the sibling keys.
    for secret in crate::email_provider::CONFIG_SECRET_FIELDS {
        if !obj.contains_key(secret) {
            continue;
        }
        let incoming = obj.get(secret).and_then(|v| v.as_str()).unwrap_or("");
        if is_masked(incoming) {
            let kept = existing
                .get(secret)
                .cloned()
                .unwrap_or(serde_json::json!(""));
            obj.insert(secret.to_string(), kept);
        }
    }
    for field in crate::email_provider::CONFIG_SECRET_FIELDS {
        obj.remove(&format!("{}_set", field));
    }

    // An empty document is not a write. Before kanban t_e14b4c36 a `{}` body answered 200 and
    // replaced the row (deleting every sibling key); now the merge would make it a no-op, so it is
    // refused instead of reported as a successful save.
    if email_config_body_is_empty(&body) {
        return Err(AppError::BadRequest(
            "Nothing to update: the request body carried no email-config keys.".to_string(),
        ));
    }

    if let Some(p) = body.get("provider").and_then(|v| v.as_str()) {
        let valid = crate::email_provider::available()
            .iter()
            .any(|v| v.get("value").and_then(|x| x.as_str()) == Some(p));
        if !valid {
            return Err(AppError::BadRequest(format!(
                "Unknown email provider '{p}'. Choose one of the values served by GET /api/v1/admin/email-config."
            )));
        }
    }

    // The credential must be SEALED before it reaches the database (kanban t_a794cb09). This row
    // is where the fleet-wide Mailgun private key lives, and it used to be written in the clear,
    // so a dump or a read-only psql handed out a working key.
    crate::email_provider::seal_config_secrets(&state.pool, &mut body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to seal email credentials: {e}")))?;

    // MERGE into the stored document, never replace it (kanban t_e14b4c36): the literal below is
    // `stored || incoming`, so a partial body touches only the keys it names.
    let stored: serde_json::Value = sqlx::query_scalar(EMAIL_CONFIG_UPSERT_SQL)
        .bind(&body)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to save email config: {e}")))?;

    Ok(Json(serde_json::json!({
        "success": true,
        // The provider ACTUALLY stored, read back from the merged row: a partial body that names no
        // provider must not answer `null` while the row still carries one.
        "provider": stored
            .get("provider")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    })))
}

/// POST /api/v1/admin/email-config/test — send a real message and return the
/// provider's true response (used by the "Send test email" button).
pub async fn test_email_config(
    State(state): State<AppState>,
    user: crate::auth::middleware::AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_platform_admin(&user)?;
    let Some(cfg) = crate::email_provider::resolve(&state.pool, None).await else {
        return Ok(Json(serde_json::json!({
            "success": false,
            "detail": "Global email provider not configured — save provider + credentials first."
        })));
    };

    let to = if user.email.trim().is_empty() {
        "swiftsoftware143@yahoo.com".to_string()
    } else {
        user.email.clone()
    };

    // The same support footer every transactional send carries (kanban t_f1931c05). An admin
    // clicking "Send test email" must see exactly what a customer's mail carries, so this body
    // goes through the ONE footer helper rather than a second, quietly drifting copy of the
    // wording. Before this, the test route bypassed `render_template` (where the footer is
    // applied) and its mail carried no support address at all.
    let (body, _) = crate::email::with_support_footer(
        Some("This is a test of the FunnelSwift system email provider.\n\nIf you received it, sending works.\n\n- FunnelSwift".to_string()),
        None,
    );

    match crate::email_provider::deliver(
        &cfg,
        &to,
        "FunnelSwift System Email Test",
        body.as_deref().unwrap_or_default(),
        None,
    )
    .await
    {
        Ok(()) => Ok(Json(serde_json::json!({
            "success": true,
            "provider": cfg.provider,
            "to": to,
            "detail": format!("{} accepted the message", cfg.provider)
        }))),
        Err(e) => Ok(Json(serde_json::json!({
            "success": false,
            "provider": cfg.provider,
            "to": to,
            "detail": e
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kanban t_e14b4c36 — the write arm must MERGE into the stored document, never replace it.
    ///
    /// The live defect (a POST of `{}` deleting `api_url` / `provider` / `from_address` from the
    /// platform's only mail config) came back because the literal was `value = $1::jsonb`: whatever
    /// the caller sent BECAME the row. This pins the semantics so a silent revert cannot pass.
    #[test]
    fn email_config_upsert_merges_the_stored_document() {
        let sql = EMAIL_CONFIG_UPSERT_SQL;
        assert!(
            sql.contains("admin_settings.value || EXCLUDED.value"),
            "the UPSERT no longer merges the stored document — a partial body would DELETE every \
             sibling key: {sql}"
        );
        assert!(
            !sql.contains("value = $1::jsonb"),
            "the UPSERT replaces the whole document again: {sql}"
        );
        assert!(
            sql.contains("jsonb_typeof(admin_settings.value) = 'object'"),
            "the merge lost its object guard — a non-object row would concatenate into an array: {sql}"
        );
        assert!(sql.contains("RETURNING value"));
    }

    /// The other half of the same defect: an empty document must not be reported as a successful
    /// save. `{}` and a body carrying only the panel's `<field>_set` scaffolding (stripped before
    /// this predicate sees it) both name nothing.
    #[test]
    fn empty_document_is_not_a_write() {
        assert!(email_config_body_is_empty(&serde_json::json!({})));
        assert!(email_config_body_is_empty(&serde_json::json!(null)));
        assert!(!email_config_body_is_empty(
            &serde_json::json!({ "from_name": "FunnelSwift" })
        ));
        assert!(!email_config_body_is_empty(
            &serde_json::json!({ "provider": "mailgun" })
        ));
    }
}
