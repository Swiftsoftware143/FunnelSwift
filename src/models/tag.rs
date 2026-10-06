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
    /// The tag → free account mapping, carried on the row so the admin panel can render it
    /// (kanban t_847f9d63). `source_app`/`plan_slug` existed since migration 067 and were
    /// seed-only because no request field could write them; this card makes all three editable.
    pub source_app: Option<String>,
    pub plan_slug: Option<String>,
    pub plan_id: Option<Uuid>,
    pub provisions_account: bool,
}

// A `SystemTag` struct (tag_name / campaign_id / webhook_url / payload_template)
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
    /// Tag → free account mapping (kanban t_847f9d63). `source_app` is the slug of the app the
    /// tag names (`coreswift`, `funnelswift`, …) and `plan_slug` the plan to seat there; both are
    /// consumed by `src/app_provision.rs` when the tag is applied to a lead.
    pub source_app: Option<String>,
    pub plan_slug: Option<String>,
    /// Execute the mapping: applying this tag mints a free account in `source_app`.
    /// Defaults FALSE — a new tag never starts provisioning by accident.
    #[serde(default)]
    pub provisions_account: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTagRequest {
    pub name: Option<String>,
    pub color: Option<String>,
    // pub description: Option<String>,
    pub group_id: Option<Uuid>,
    /// kanban t_f94a8a00: an explicit "remove this tag from its group" request.
    ///
    /// `group_id` alone cannot express a clear: `update_tag()` binds `req.group_id.or(tag.group_id)`,
    /// so an ABSENT key and an explicit `null` both mean "keep the stored group" (the shape every
    /// shipped caller already sends), and a bare `""` is an axum 422 `UUID parsing failed`. Rather
    /// than change what `null` means — which would silently wipe the group of any caller that sends
    /// `group_id: null` on a plain rename — the clear is this ADDITIVE flag, so every existing body
    /// shape keeps meaning exactly what it means today. `clear_group: true` ungroups the tag; when it
    /// is true and `group_id` is also present, the clear wins.
    #[serde(default)]
    pub clear_group: Option<bool>,
    pub is_system: Option<bool>,
    pub metadata: Option<serde_json::Value>,
    /// The tag → free account mapping (kanban t_847f9d63), editable in the admin System Tags
    /// editor. An ABSENT key means "keep the stored value" for all three.
    ///
    /// For `source_app` / `plan_slug` an explicitly sent EMPTY string means CLEAR (stored NULL):
    /// the admin form always posts the field's own value, so "" is the operator emptying the box —
    /// the same precedent `group_id` set (a value that can be SET but never CLEARED is a defect).
    /// `provisions_account` is a plain boolean: absent keeps, `false` really means false, which is
    /// how the "Auto-provision free account" toggle turns a shipped-ON mapping back OFF.
    pub source_app: Option<String>,
    pub plan_slug: Option<String>,
    pub provisions_account: Option<bool>,
}
