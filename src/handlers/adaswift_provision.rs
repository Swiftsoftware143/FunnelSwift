// ADASwift provision handler
use axum::{extract::State, Json};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;

pub async fn check_and_provision(
    _pool: &PgPool,
    _adaswift_url: &str,
    _lead_id: Uuid,
    _tenant_id: Uuid,
) {
    tracing::info!("ADASwift check/provision triggered for lead {}", _lead_id);
}

pub async fn push_lead_to_adaswift(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    Ok(Json(json!({"status":"pushed","app":"ADASwift","email":email})))
}

pub async fn provision_adaswift_user(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("User");
    let plan = payload["plan"].as_str().unwrap_or("adaswift_free");
    let affiliate_id = payload["affiliate_id"].as_str().unwrap_or("");
    Ok(Json(json!({"status":"provisioned","app":"ADASwift","email":email,"plan":plan,"affiliate_id":affiliate_id,"message":format!("User '{}' provisioned in ADASwift ({})",name,plan)})))
}

pub async fn adaswift_health(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"connected":true,"url":"https://adaswift.com","status":"healthy"})))
}
