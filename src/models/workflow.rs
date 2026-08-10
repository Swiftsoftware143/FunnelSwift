use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Workflow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub trigger_type: String,
    pub trigger_config: Option<serde_json::Value>,
    pub actions: serde_json::Value,
    pub is_active: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub run_count: i32,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateWorkflowRequest {
    #[validate(length(min = 1, message = "Name is required"))]
    pub name: String,
    pub description: Option<String>,
    #[validate(length(min = 1, message = "Trigger type is required"))]
    pub trigger_type: String,
    pub trigger_config: Option<serde_json::Value>,
    pub actions: serde_json::Value,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateWorkflowRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub trigger_type: Option<String>,
    pub trigger_config: Option<serde_json::Value>,
    pub actions: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub trigger_type: String,
    pub trigger_config: Option<serde_json::Value>,
    pub actions: serde_json::Value,
    pub is_active: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub run_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteWorkflowRequest {
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ExecuteWorkflowResponse {
    pub execution_id: Uuid,
    pub status: String,
    pub started_at: DateTime<Utc>,
}
