use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn store_linkedin_auth(auth: AuthUser, State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "LinkedIn auth stored"})))
}
pub async fn get_linkedin_auth_status(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"status": "not_configured"})))
}
pub async fn delete_linkedin_auth(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "LinkedIn auth deleted"})))
}
pub async fn get_linkedin_cookies_for_user(auth: AuthUser, State(state): State<AppState>, Path(user_id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"cookies": []})))
}
