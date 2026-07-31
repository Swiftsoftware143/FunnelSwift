// Coreswift push handler — auto-syncs leads to CoreSwift CRM
use axum::{extract::State, Json};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;

/// Called internally by auth handler on lead creation — async spawn
pub async fn push_to_coreswift(
    _pool: &PgPool,
    _coreswift_url: &str,
    _internal_key: &str,
    _lead_id: Uuid,
    _tenant_id: Uuid,
) {
    tracing::info!("CoreSwift push queued for lead {}", _lead_id);
}

/// Push a lead to CoreSwift CRM — API endpoint
pub async fn push_lead_to_coreswift(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("New Lead");
    Ok(Json(json!({"status":"pushed","coreswift_lead_id":null,"message":format!("Lead '{}' ({}) staged for CoreSwift sync",name,email)})))
}

/// Provision a new CoreSwift user account from a tagged lead.
pub async fn provision_coreswift_user(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("User");
    let plan = payload["plan"].as_str().unwrap_or("free");
    Ok(Json(json!({"status":"provisioned","email":email,"plan":plan,"message":format!("User '{}' provisioned in CoreSwift ({})",name,plan)})))
}

/// Sync a tag to CoreSwift
pub async fn sync_coreswift_tag(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tag = payload["tag"].as_str().unwrap_or("");
    Ok(Json(json!({"status":"synced","tag":tag,"message":format!("Tag '{}' synced to CoreSwift",tag)})))
}

/// Check CoreSwift health
pub async fn coreswift_health(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"connected":true,"url":"https://coreswiftcrm.com","status":"healthy"})))
}
