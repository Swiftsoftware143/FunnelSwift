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
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct WebhookDeliveryLog {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event: String,
    pub status: String,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub delivered_at: NaiveDateTime,
}
