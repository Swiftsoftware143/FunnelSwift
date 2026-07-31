use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::error::AppResult;
use crate::state::AppState;

pub async fn sync_plan_tag(State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"synced": true, "message": "Plan-tag sync triggered"})))
}
