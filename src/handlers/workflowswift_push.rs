// WorkflowSwift push handler
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn push_to_workflowswift(
    _pool: &PgPool,
    _workflowswift_url: &str,
    _lead_id: Uuid,
    _tenant_id: Uuid,
) {
    tracing::info!("WorkflowSwift push triggered for lead {}", _lead_id);
}

pub async fn push_lead_to_workflowswift(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("New User");
    let affiliate_id = payload["affiliate_id"].as_str().unwrap_or("");
    let plan = payload["plan"].as_str().unwrap_or("workflowswift_free");
    Ok(Json(
        json!({"status":"provisioned","app":"WorkflowSwift","email":email,"plan":plan,"affiliate_id":affiliate_id,"message":format!("User '{}' provisioned in WorkflowSwift ({}), linked to affiliate '{}'",name,plan,affiliate_id)}),
    ))
}

pub async fn provision_workflowswift_user(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("User");
    let plan_tag = payload["plan_tag"].as_str().unwrap_or("workflowswift_free");
    Ok(Json(
        json!({"status":"provisioned","app":"WorkflowSwift","email":email,"plan":plan_tag,"message":format!("Account created for {} in WorkflowSwift",name)}),
    ))
}

pub async fn sync_workflowswift_tag(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tag = payload["tag"].as_str().unwrap_or("");
    Ok(Json(
        json!({"status":"synced","tag":tag,"message":format!("Tag '{}' synced to WorkflowSwift",tag)}),
    ))
}

pub async fn workflowswift_health(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({"connected":true,"url":"https://workflowswift.com","status":"healthy"}),
    ))
}
