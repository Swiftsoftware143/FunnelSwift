use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct PlanTagMapping {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub tag_id: Uuid,
    pub source_software: Option<String>,
    pub target_software: Option<String>,
    pub commission_rates: Option<serde_json::Value>,
    pub allow_dual_routing: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncTagMappingRequest {
    pub plan_id: Uuid,
    pub tag_ids: Vec<Uuid>,
    pub source_software: Option<String>,
    pub target_software: Option<String>,
    pub commission_rates: Option<serde_json::Value>,
    pub allow_dual_routing: Option<bool>,
}
