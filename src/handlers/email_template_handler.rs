//! Email Template Admin API
//! Manage email templates (welcome, password_reset, purchase_confirmed, etc.)

use axum::{extract::State, http::StatusCode, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

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
        serde_json::json!({"type": "welcome", "description": "Sent after user registration", "merge_fields": ["name", "email", "password", "login_url", "app_name"]}),
        serde_json::json!({"type": "password_reset", "description": "Sent when user requests password reset", "merge_fields": ["name", "token", "app_name"]}),
        serde_json::json!({"type": "purchase_confirmed", "description": "Sent after successful payment", "merge_fields": ["name", "plan_name", "login_url", "app_name"]}),
    ])
}

/// GET /api/v1/admin/email-config
pub async fn get_email_config() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "provider": "mailgun",
        "api_url": std::env::var("EMAIL_API_URL").unwrap_or_default(),
        "from_address": std::env::var("EMAIL_FROM").unwrap_or_default(),
        "api_key_configured": std::env::var("EMAIL_API_KEY").map(|k| !k.is_empty()).unwrap_or(false),
    }))
}

/// POST /api/v1/admin/email-config
pub async fn update_email_config(Json(req): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let provider = req["provider"].as_str().unwrap_or("mailgun");
    let api_url = req["api_url"].as_str().unwrap_or("");
    let api_key = req["api_key"].as_str().unwrap_or("");
    let from_address = req["from_address"].as_str().unwrap_or("");

    if !api_url.is_empty() {
        std::env::set_var("EMAIL_API_URL", api_url);
    }
    if !api_key.is_empty() {
        std::env::set_var("EMAIL_API_KEY", api_key);
    }
    if !from_address.is_empty() {
        std::env::set_var("EMAIL_FROM", from_address);
    }

    Json(serde_json::json!({
        "status": "configured",
        "provider": provider,
        "note": "For permanent changes, update /etc/swift/env/funnelswift.env and restart the service"
    }))
}
