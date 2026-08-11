use axum::{extract::State, Json};
use crate::error::AppResult;
use crate::state::AppState;

pub async fn get_site(State(_state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({"status": "ok"})))
}

pub async fn update_site(State(_state): State<AppState>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({"status": "ok"})))
}
