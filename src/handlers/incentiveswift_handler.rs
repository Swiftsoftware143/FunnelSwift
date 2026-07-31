use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;

pub async fn get_incentiveswift_config(_auth: AuthUser, State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"api_url": "https://incentiveswift.com", "enabled": true})))
}
