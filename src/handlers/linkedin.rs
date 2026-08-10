use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};

pub async fn handle_linkedin_lookup(
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({"full_name": payload["name"].as_str().unwrap_or(""), "headline": "", "profile_url": "", "found": false}),
    ))
}
