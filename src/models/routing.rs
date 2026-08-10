use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct TargetSoftware {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub webhook_url: String,
    pub api_key: Option<String>,
    pub portfolio_company_id: Option<Uuid>,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
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
    pub created_at: DateTime<Utc>,
}
