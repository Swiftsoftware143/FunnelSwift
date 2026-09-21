use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A routing target as it exists IN THE DATABASE — `stored_api_key` is the value as stored, i.e.
/// `enc:v1:` ciphertext (src/security/provider_key_crypto.rs). This struct is deliberately NOT
/// `Serialize`: a handler that hands it to a client would echo a credential. The read path
/// (routing_handler::list_target_software) decrypts it and exposes `api_key_masked` only.
#[derive(Debug, FromRow)]
pub struct TargetSoftware {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub webhook_url: String,
    #[sqlx(rename = "api_key")]
    pub stored_api_key: Option<String>,
    pub portfolio_company_id: Option<Uuid>,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTargetSoftwareRequest {
    pub name: String,
    pub webhook_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct RoutingLog {
    pub id: Uuid,
    pub lead_id: Uuid,
    pub source_tenant: Uuid,
    pub target_software: Uuid,
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
}
