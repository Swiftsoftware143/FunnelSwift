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

// A `SystemTag` struct (tag_name / target_software / campaign_id / webhook_url / payload_template)
// used to live here, together with `TagAssignmentResult`, `WebhookResult`, `WebhookPayload`,
// `ContactPayload` and `TagPayload`. All six were referenced by nothing but their own definitions:
// there is no `system_tags` table in the `funnelswift` DB, no per-tag target in the System Tags UI,
// and no code path that acts on one, so the structs only described a cross-app auto-provisioning
// feature that was never built and is not wanted (kanban t_75bb53e7; the push legs they implied were
// removed in t_0a6a93f1, and src/api_router.rs carries the do-not-re-add note). Removed rather than
// left as a wire-me-up invitation. `tags.is_system` remains: admin-created tags shared with every
// tenant, managed through src/handlers/tag_handler.rs.

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
    pub is_system: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTagRequest {
    pub name: Option<String>,
    pub color: Option<String>,
    // pub description: Option<String>,
    pub group_id: Option<Uuid>,
    pub is_system: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}
