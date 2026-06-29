use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    // pub description: Option<String>,
    pub is_system: bool,
    pub group_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    // pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SystemTag {
    pub id: Uuid,
    pub tag_name: String,
    pub target_software: String,  // 'crm-swift', 'workflowswift', 'adaswift', 'sendiio', 'webhook', etc.
    pub campaign_id: Option<String>,  // Campaign/list ID in target system
    pub webhook_url: Option<String>,  // For custom webhooks
    pub payload_template: Option<serde_json::Value>,  // Template for webhook payload
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    // pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactTag {
    pub contact_id: Uuid,
    pub tag_id: Uuid,
    pub tagged_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AssignTagRequest {
    pub contact_id: Uuid,
    pub tag_name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTagRequest {
    pub name: String,
    pub color: Option<String>,
    // pub description: Option<String>,
    pub group_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTagRequest {
    pub name: Option<String>,
    pub color: Option<String>,
    // pub description: Option<String>,
    pub group_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct TagAssignmentResult {
    pub contact_id: Uuid,
    pub tag_name: String,
    pub triggered_webhooks: Vec<WebhookResult>,
}

#[derive(Debug, Serialize)]
pub struct WebhookResult {
    pub target_software: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub contact: ContactPayload,
    pub tag: TagPayload,
    pub source: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ContactPayload {
    pub id: Uuid,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub custom_fields: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct TagPayload {
    pub name: String,
    pub campaign_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}