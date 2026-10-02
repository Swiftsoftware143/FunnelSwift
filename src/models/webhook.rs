use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Webhook {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub url: String,
    pub events: serde_json::Value,
    pub secret: Option<String>,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub url: String,
    pub events: Vec<String>,
    pub secret: Option<String>,
    /// Optional on create: the console's own Active toggle. Absent means the column default (true).
    /// Ignored by serde before kanban t_ae907da8, so the toggle on the Add form was decorative.
    #[serde(default)]
    pub is_active: Option<bool>,
}

/// The Webhook editor's Save (kanban t_ae907da8): `PUT /api/v1/webhooks/:id` was NOT routed
/// (DELETE only — src/api_router.rs), so the console's editor could never save. Every field is
/// optional and an absent key KEEPS the stored value, so a no-touch save is a no-op that still
/// answers 200. `is_active` is the column's own name — the console used to read `w.active`, a key
/// `Webhook` has never serialized, so every row (and the editor's toggle) read as active.
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateWebhookRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

/// One row of `webhook_delivery_log`. `attempt` is the attempt this row is recording and
/// `delivered_at` is the time of that attempt, so the pair is what a delivery-log view prints
/// ("attempt 2 of 3, failed 12s ago"). kanban t_431faa99: the table was created by
/// `migrations/0016_webhook_delivery_log.sql` with a retry index and a writer never existed.
#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct WebhookDelivery {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub event: String,
    pub status: String,
    pub status_code: Option<i32>,
    pub attempt: i32,
    pub max_attempts: i32,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub delivered_at: NaiveDateTime,
    pub next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
}
