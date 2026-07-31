use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use crate::error::AppResult;
use crate::state::AppState;

pub async fn public_signup(State(_state): State<AppState>, Json(payload): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(json!({"message": "Signup endpoint", "email": payload["email"]}))))
}
