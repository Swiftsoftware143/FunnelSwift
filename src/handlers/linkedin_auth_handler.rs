use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};

pub async fn store_linkedin_auth(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "LinkedIn auth stored"})))
}
pub async fn get_linkedin_auth_status(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"status": "not_configured"})))
}
pub async fn delete_linkedin_auth(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "LinkedIn auth deleted"})))
}
pub async fn get_linkedin_cookies_for_user(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_user_id): Path<String>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"cookies": []})))
}
