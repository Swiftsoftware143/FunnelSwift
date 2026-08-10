use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};

pub async fn handle_parse_card(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({"name": "", "title": "", "company": "", "email": "", "phone": "", "parsed": false}),
    ))
}
