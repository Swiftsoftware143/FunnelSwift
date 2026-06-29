use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub key_hash: String,
    pub prefix: String,
    pub permissions: serde_json::Value,
    pub target_url: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub prefix: String,
    pub target_url: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyCreateResponse {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub raw_key: String,
    pub target_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: Option<String>,
    pub target_url: Option<String>,
    pub permissions: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub target_url: Option<String>,
    pub permissions: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

impl ApiKey {
    pub fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            tenant_id: row.try_get("tenant_id")?,
            user_id: row.try_get("user_id")?,
            name: row.try_get("name")?,
            key_hash: row.try_get("key_hash")?,
            prefix: row.try_get("prefix")?,
            permissions: row.try_get("permissions")?,
            target_url: row.try_get("target_url")?,
            last_used_at: row.try_get("last_used_at")?,
            expires_at: row.try_get("expires_at")?,
            is_active: row.try_get("is_active")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

impl From<ApiKey> for ApiKeyResponse {
    fn from(k: ApiKey) -> Self {
        Self {
            id: k.id,
            tenant_id: k.tenant_id,
            user_id: k.user_id,
            name: k.name,
            prefix: k.prefix,
            target_url: k.target_url,
            last_used_at: k.last_used_at,
            expires_at: k.expires_at,
            is_active: k.is_active,
            created_at: k.created_at,
        }
    }
}
