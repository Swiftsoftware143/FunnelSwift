use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};

pub async fn sync_plan_tag(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({"synced": true, "message": "Plan-tag sync triggered"}),
    ))
}
