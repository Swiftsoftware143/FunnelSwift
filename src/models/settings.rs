use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SiteSettings {
    pub id: Uuid,
    pub key: String,
    pub value: serde_json::Value,
    pub description: Option<String>,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub key: String,
    pub value: serde_json::Value,
    pub description: Option<String>,
}
