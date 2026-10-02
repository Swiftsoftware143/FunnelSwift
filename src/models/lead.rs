use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Lead {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    // NULLABLE-DECODED-AS-NON-OPTION (struct), kanban t_d5da34d0. `leads.status` is NULLABLE
    // (DEFAULT 'new') and `Lead` is decoded by FOUR `SELECT *` statements (list / by-id / the
    // update read / export), so a single NULL status failed the whole-row decode of all four with
    // "unexpected null; try decoding as an Option". Option is the arm this shape forces: a
    // `SELECT *` cannot carry a COALESCE and there is no expression item to wrap. No writer can
    // produce the NULL (create omits the column so the DEFAULT applies, the affiliate and
    // web-capture inserts bind the literal 'new', the update path falls back to "active"), so this
    // only changes the JSON of a state the app cannot reach - and the SPA already guards the key
    // (`l.status || ""`, `l.status && ...`).
    pub status: Option<String>,
    pub stage: Option<String>,
    pub source: Option<String>,
    pub tags: Option<serde_json::Value>,
    pub notes: Option<String>,
    pub assigned_to: Option<Uuid>,
    pub score: Option<i32>,
    pub custom_fields: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateLeadRequest {
    pub name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    #[validate(email(message = "Invalid email format"))]
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub status: Option<String>,
    pub stage: Option<String>,
    pub source: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub assigned_to: Option<Uuid>,
    pub score: Option<i32>,
    pub custom_fields: Option<serde_json::Value>,
    /// kanban t_d4ce013c. The served lead form's tag picker collects the picked tags BY ID
    /// (`www-app/index.html` `SL()`: `document.querySelectorAll(".ts[data-tid]")` -> `tag_ids`), so
    /// that is the key the write path has to bind. It was accepted by NEITHER lead route and nothing
    /// denies unknown fields, so serde dropped it silently: the picker offered the system tags (GET
    /// /api/v1/tags), the save answered 201, and the tags were never stored. `create_lead` now
    /// resolves these ids to names with the same predicate `POST /api/v1/leads/:id/tags` uses
    /// (`tenant_id = $1 OR is_system = true`, the 669a2a7 fix) and runs them through the one
    /// `apply_lead_tags`, inside the same request — no second round trip.
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateLeadRequest {
    pub name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    #[validate(email(message = "Invalid email format"))]
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub status: Option<String>,
    pub stage: Option<String>,
    pub source: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub assigned_to: Option<Uuid>,
    pub score: Option<i32>,
    pub custom_fields: Option<serde_json::Value>,
    /// kanban t_d4ce013c — the update half of the same defect on `CreateLeadRequest::tag_ids`: the
    /// served Edit modal posts `tag_ids` (the ids the picker collected) on PUT /api/v1/leads/:id and
    /// the key was silently dropped, so the save answered 200 with the tags unchanged. `update_lead`
    /// resolves these ids to names and runs them through the ONE `apply_lead_tags` (the function
    /// `POST /api/v1/leads/:id/tags` is made of), after its own UPDATE — which on this path rebinds
    /// the STORED `tags` value (the modal sends no `tags`), so `apply_lead_tags` stays the single
    /// semantic writer of a new tag list for this route.
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize)]
pub struct LeadResponse {
    pub id: Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub status: String,
    pub stage: Option<String>,
    pub source: Option<String>,
    pub score: Option<i32>,
    pub tags: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AssignRequest {
    pub assigned_to: Uuid,
}

/// `POST /api/v1/leads/:id/stage` — the PIPELINE column, not the status. `leads.stage` holds one of
/// the tenant's `tenant_settings.lead_stages` values (Title-case, edited on the "Lead Stages"
/// screen); `leads.status` is a different five-value lowercase lifecycle, decided in kanban
/// t_adf187f9.
#[derive(Debug, Deserialize)]
pub struct StageRequest {
    pub stage: String,
}

/// `PUT /api/v1/leads/:id/status` (kanban t_2f2c184b) — the leads row badge's "Change Status"
/// quick-change control. `leads.status` is its own column; this is NOT the `stage` write.
/// kanban t_adf187f9: the canonical space is the five lowercase values the served shell declares in
/// `LD_STATUS_DEFAULT` (new, contacted, qualified, converted, lost); this route stays vocabulary-free
/// on purpose, see `lead_handler::update_lead_status`.
#[derive(Debug, Deserialize)]
pub struct LeadStatusRequest {
    pub status: String,
}
