use axum::{extract::{Path, Query, State}, Json};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
use reqwest;

use crate::{
    auth::AuthUser,
    db::Database,
    error::{AppError, Result},
    models::tag::{
        AssignTagRequest, ContactPayload, ContactTag, SystemTag, Tag, 
        TagAssignmentResult, TagPayload, WebhookPayload, WebhookResult
    },
    models::lead::Lead,
    models::PaginatedResponse,
};

#[derive(Debug, Deserialize)]
pub struct ListTagsQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
    pub search: Option<String>,
}

pub async fn list_tags(
    State(db): State<Database>,
    user: AuthUser,
    Query(query): Query<ListTagsQuery>,
) -> Result<Json<PaginatedResponse<Tag>>> {
    let page = query.page.unwrap_or(1);
    let per_page = query.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let tenant_id = user.tenant_id
        .and_then(|t| Uuid::parse_str(&t).ok())
        .ok_or_else(|| AppError::Auth("Tenant not found".to_string()))?;

    let tags = sqlx::query_as!(
        Tag,
        r#"SELECT * FROM tags 
         WHERE tenant_id = $1 
         AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%')
         ORDER BY created_at DESC
         LIMIT $3 OFFSET $4"#,
        tenant_id,
        query.search,
        per_page as i64,
        offset as i64
    )
    .fetch_all(db.pool())
    .await?;

    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM tags WHERE tenant_id = $1"#,
        tenant_id
    )
    .fetch_one(db.pool())
    .await?
    .unwrap_or(0);

    let total_pages = (total as f64 / per_page as f64).ceil() as i32;

    Ok(Json(PaginatedResponse {
        data: tags,
        total,
        page,
        per_page,
        total_pages,
    }))
}

pub async fn create_tag(
    State(db): State<Database>,
    user: AuthUser,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<Tag>> {
    let tenant_id = user.tenant_id
        .and_then(|t| Uuid::parse_str(&t).ok())
        .ok_or_else(|| AppError::Auth("Tenant not found".to_string()))?;

    let name = payload["name"].as_str()
        .ok_or_else(|| AppError::Validation("Tag name required".to_string()))?;
    let color = payload["color"].as_str();
    let description = payload["description"].as_str();

    let tag = sqlx::query_as!(
        Tag,
        r#"INSERT INTO tags (tenant_id, name, color, description, is_system)
         VALUES ($1, $2, $3, $4, false)
         RETURNING *"#,
        tenant_id,
        name,
        color,
        description
    )
    .fetch_one(db.pool())
    .await?;

    Ok(Json(tag))
}

/// Assign a tag to a contact and trigger associated webhooks
pub async fn assign_tag(
    State(db): State<Database>,
    user: AuthUser,
    Json(req): Json<AssignTagRequest>,
) -> Result<Json<TagAssignmentResult>> {
    let tenant_id = user.tenant_id
        .and_then(|t| Uuid::parse_str(&t).ok())
        .ok_or_else(|| AppError::Auth("Tenant not found".to_string()))?;

    // Get contact details
    let contact = sqlx::query_as!(
        Lead,
        r#"SELECT * FROM leads WHERE id = $1 AND tenant_id = $2"#,
        req.contact_id,
        tenant_id
    )
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| AppError::NotFound("Contact not found".to_string()))?;

    // Get or create tag
    let tag = sqlx::query_as!(
        Tag,
        r#"INSERT INTO tags (tenant_id, name, is_system)
         VALUES ($1, $2, false)
         ON CONFLICT (tenant_id, name) DO UPDATE SET updated_at = NOW()
         RETURNING *"#,
        tenant_id,
        req.tag_name
    )
    .fetch_one(db.pool())
    .await?;

    // Assign tag to contact
    sqlx::query!(
        r#"INSERT INTO contact_tags (contact_id, tag_id, tagged_by)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING"#,
        req.contact_id,
        tag.id,
        Uuid::parse_str(&user.id).ok()
    )
    .execute(db.pool())
    .await?;

    // Check for system tag triggers
    let system_tag = sqlx::query_as!(
        SystemTag,
        r#"SELECT * FROM system_tags 
         WHERE tag_name = $1 AND is_active = true"#,
        req.tag_name
    )
    .fetch_optional(db.pool())
    .await?;

    let mut webhook_results = Vec::new();

    if let Some(system_tag) = system_tag {
        // Trigger webhook based on target software
        let result = trigger_webhook(&contact, &system_tag).await;
        webhook_results.push(result);

        // Log webhook event
        sqlx::query!(
            r#"INSERT INTO webhook_events 
             (contact_id, tag_id, tag_name, target_software, campaign_id, payload, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            req.contact_id,
            system_tag.id,
            req.tag_name,
            system_tag.target_software,
            system_tag.campaign_id,
            json!(contact),
            if webhook_results.iter().all(|r| r.success) { "success" } else { "failed" }
        )
        .execute(db.pool())
        .await?;
    }

    Ok(Json(TagAssignmentResult {
        contact_id: req.contact_id,
        tag_name: req.tag_name,
        triggered_webhooks: webhook_results,
    }))
}

async fn trigger_webhook(contact: &Lead, system_tag: &SystemTag) -> WebhookResult {
    let payload = WebhookPayload {
        contact: ContactPayload {
            id: contact.id,
            email: contact.email.clone(),
            phone: contact.phone.clone(),
            first_name: contact.first_name.clone(),
            last_name: contact.last_name.clone(),
            company: contact.company.clone(),
            custom_fields: contact.custom_fields.clone(),
        },
        tag: TagPayload {
            name: system_tag.tag_name.clone(),
            campaign_id: system_tag.campaign_id.clone(),
            metadata: system_tag.payload_template.clone(),
        },
        source: "funnelswift".to_string(),
        timestamp: chrono::Utc::now(),
    };

    let webhook_url = match system_tag.target_software.as_str() {
        "crm-swift" => std::env::var("CRM_SWIFT_WEBHOOK_URL").ok(),
        "coreswift" => std::env::var("CORESWIFT_WEBHOOK_URL").ok(),
        "incentiveswift" => std::env::var("INCENTIVESWIFT_WEBHOOK_URL").ok(),
        "workflowswift" => std::env::var("WORKFLOWSWIFT_WEBHOOK_URL").ok(),
        "adaswift" => std::env::var("ADASWIFT_WEBHOOK_URL").ok(),
        "missedcall" => std::env::var("MISSEDCALL_WEBHOOK_URL").ok(),
        "sendiio" => std::env::var("SENDIIO_WEBHOOK_URL").ok(),
        "webhook" => system_tag.webhook_url.clone(),
        _ => None,
    };

    let Some(url) = webhook_url else {
        return WebhookResult {
            target_software: system_tag.target_software.clone(),
            success: false,
            message: "Webhook URL not configured".to_string(),
        };
    };

    let client = reqwest::Client::new();
    match client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("X-Event-Source", "funnelswift")
        .header("X-Internal-Key", &std::env::var("INTERNAL_SYNC_KEY").unwrap_or_default())
        .json(&payload)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                WebhookResult {
                    target_software: system_tag.target_software.clone(),
                    success: true,
                    message: format!("Webhook sent successfully: {}", status),
                }
            } else {
                WebhookResult {
                    target_software: system_tag.target_software.clone(),
                    success: false,
                    message: format!("Webhook failed: {}", status),
                }
            }
        }
        Err(e) => WebhookResult {
            target_software: system_tag.target_software.clone(),
            success: false,
            message: format!("Webhook error: {}", e),
        },
    }
}

/// Get all tags for a contact
pub async fn get_contact_tags(
    State(db): State<Database>,
    user: AuthUser,
    Path(contact_id): Path<Uuid>,
) -> Result<Json<Vec<Tag>>> {
    let tenant_id = user.tenant_id
        .and_then(|t| Uuid::parse_str(&t).ok())
        .ok_or_else(|| AppError::Auth("Tenant not found".to_string()))?;

    // Verify contact belongs to tenant
    let _contact = sqlx::query_as!(
        Lead,
        r#"SELECT * FROM leads WHERE id = $1 AND tenant_id = $2"#,
        contact_id,
        tenant_id
    )
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| AppError::NotFound("Contact not found".to_string()))?;

    let tags = sqlx::query_as!(
        Tag,
        r#"SELECT t.* FROM tags t
         JOIN contact_tags ct ON t.id = ct.tag_id
         WHERE ct.contact_id = $1"#,
        contact_id
    )
    .fetch_all(db.pool())
    .await?;

    Ok(Json(tags))
}

/// Remove tag from contact
pub async fn remove_tag_from_contact(
    State(db): State<Database>,
    user: AuthUser,
    Path((contact_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>> {
    let tenant_id = user.tenant_id
        .and_then(|t| Uuid::parse_str(&t).ok())
        .ok_or_else(|| AppError::Auth("Tenant not found".to_string()))?;

    sqlx::query!(
        r#"DELETE FROM contact_tags 
         WHERE contact_id = $1 AND tag_id = $2"#,
        contact_id,
        tag_id
    )
    .execute(db.pool())
    .await?;

    Ok(Json(json!({ 
        "message": "Tag removed from contact",
        "contact_id": contact_id,
        "tag_id": tag_id
    })))
}