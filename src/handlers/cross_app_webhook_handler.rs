use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};

pub async fn handle_conversion_webhook(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::OK, Json(json!({"received": true}))))
}
pub async fn track_lead_conversion(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"tracked": true})))
}
