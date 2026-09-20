//! Email Template Admin API
//! Manage email templates (welcome, password_reset, purchase_confirmed, etc.)

use axum::{extract::State, http::StatusCode, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// The one string a masked secret is replaced with in responses.
const MASK: &str = "••••••••";

fn is_masked(v: &str) -> bool {
    v.is_empty() || v.chars().all(|c| c == '•' || c == '*')
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
    pub name: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub html_body: Option<String>,
    pub is_default: Option<bool>,
}

/// GET /api/v1/admin/email-templates
pub async fn list_templates(
    State(state): State<AppState>,
) -> Result<Json<Vec<EmailTemplate>>, AppError> {
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
    State(state): State<AppState>,
    Json(req): Json<CreateTemplateRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO email_templates (id, template_type, name, subject, body, html_body, is_default) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(id)
    .bind(&req.template_type)
    .bind(&req.name)
    .bind(&req.subject)
    .bind(&req.body)
    .bind(&req.html_body)
    .bind(req.is_default.unwrap_or(false))
    .execute(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to create template: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id, "status": "created"})),
    ))
}

/// PUT /api/v1/admin/email-templates/:id
pub async fn update_template(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<UpdateTemplateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let existing = sqlx::query(
        "SELECT name, subject, body, html_body, is_default FROM email_templates WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
    .ok_or_else(|| AppError::NotFound("Template not found".into()))?;

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

    sqlx::query("UPDATE email_templates SET name=$1, subject=$2, body=$3, html_body=$4, is_default=$5, updated_at=NOW() WHERE id=$6")
        .bind(&name)
        .bind(&subject)
        .bind(&body)
        .bind(&html_body)
        .bind(is_default)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update template: {e}")))?;

    Ok(Json(serde_json::json!({"id": id, "status": "updated"})))
}

/// DELETE /api/v1/admin/email-templates/:id
pub async fn delete_template(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query("DELETE FROM email_templates WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to delete template: {e}")))?;

    Ok(Json(serde_json::json!({"id": id, "status": "deleted"})))
}

/// GET /api/v1/admin/email-templates/:id
pub async fn get_template(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<EmailTemplate>, AppError> {
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
pub async fn list_template_types() -> Json<Vec<serde_json::Value>> {
    Json(vec![
        serde_json::json!({"type": "welcome", "description": "Sent after user registration", "merge_fields": ["name", "email", "login_url", "app_name"]}),
        serde_json::json!({"type": "password_reset", "description": "Sent when user requests password reset", "merge_fields": ["name", "token", "app_name"]}),
        serde_json::json!({"type": "purchase_confirmed", "description": "Sent after successful payment", "merge_fields": ["name", "plan_name", "login_url", "app_name"]}),
    ])
}

/// GET /api/v1/admin/email-config — global (system mail) provider config, secrets masked.
/// DB-backed: nothing here reads the process environment.
pub async fn get_email_config(State(state): State<AppState>) -> Json<serde_json::Value> {
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
        if obj.contains_key("api_key") {
            let set = cfg.as_ref().map(|c| !c.api_key.is_empty()).unwrap_or(false);
            obj.insert(
                "api_key".into(),
                serde_json::json!(if set { MASK } else { "" }),
            );
            obj.insert("api_key_set".into(), serde_json::json!(set));
        }
        if obj.contains_key("smtp_password") {
            let set = cfg
                .as_ref()
                .map(|c| !c.smtp_password.is_empty())
                .unwrap_or(false);
            obj.insert(
                "smtp_password".into(),
                serde_json::json!(if set { MASK } else { "" }),
            );
            obj.insert("smtp_password_set".into(), serde_json::json!(set));
        }
    }

    Json(serde_json::json!({
        "config": out,
        "configured": cfg.as_ref().map(|c| c.is_configured()).unwrap_or(false),
        "provider": cfg.as_ref().map(|c| c.provider.clone()).unwrap_or_default(),
        "providers": crate::email_provider::available(),
    }))
}

/// POST /api/v1/admin/email-config — save the global system-mail provider.
/// A masked secret coming back from the UI never overwrites the stored one.
pub async fn update_email_config(
    State(state): State<AppState>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let existing: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM admin_settings WHERE key = 'email'")
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let existing = existing.unwrap_or_else(|| serde_json::json!({}));

    let obj = body
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("Expected a JSON object".to_string()))?;

    for secret in ["api_key", "smtp_password"] {
        let incoming = obj.get(secret).and_then(|v| v.as_str()).unwrap_or("");
        if is_masked(incoming) {
            let kept = existing
                .get(secret)
                .cloned()
                .unwrap_or(serde_json::json!(""));
            obj.insert(secret.to_string(), kept);
        }
    }
    obj.remove("api_key_set");
    obj.remove("smtp_password_set");

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

    sqlx::query(
        "INSERT INTO admin_settings (key, value, description, updated_at)
         VALUES ('email', $1::jsonb, 'Global system email provider (admin-editable)', NOW())
         ON CONFLICT (key) DO UPDATE SET value = $1::jsonb, updated_at = NOW()",
    )
    .bind(&body)
    .execute(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to save email config: {e}")))?;

    Ok(Json(
        serde_json::json!({ "success": true, "provider": body.get("provider") }),
    ))
}

/// POST /api/v1/admin/email-config/test — send a real message and return the
/// provider's true response (used by the "Send test email" button).
pub async fn test_email_config(
    State(state): State<AppState>,
    user: crate::auth::middleware::AuthUser,
) -> Json<serde_json::Value> {
    let Some(cfg) = crate::email_provider::resolve(&state.pool, None).await else {
        return Json(serde_json::json!({
            "success": false,
            "detail": "Global email provider not configured — save provider + credentials first."
        }));
    };

    let to = if user.email.trim().is_empty() {
        "swiftsoftware143@yahoo.com".to_string()
    } else {
        user.email.clone()
    };

    match crate::email_provider::deliver(
        &cfg,
        &to,
        "FunnelSwift System Email Test",
        "This is a test of the FunnelSwift system email provider.\n\nIf you received it, sending works.\n\n- FunnelSwift",
        None,
    )
    .await
    {
        Ok(()) => Json(serde_json::json!({
            "success": true,
            "provider": cfg.provider,
            "to": to,
            "detail": format!("{} accepted the message", cfg.provider)
        })),
        Err(e) => Json(serde_json::json!({
            "success": false,
            "provider": cfg.provider,
            "to": to,
            "detail": e
        })),
    }
}
