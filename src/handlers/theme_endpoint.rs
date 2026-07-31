use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;

pub async fn list_themes(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([{"id":"default","name":"Default","primary":"#6366f1"},{"id":"dark","name":"Dark Mode","primary":"#8b5cf6"},{"id":"ocean","name":"Ocean","primary":"#0ea5e9"}])))
}
pub async fn list_templates(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([{"id":"biolink","name":"Bio Link"},{"id":"business-card","name":"Business Card"},{"id":"minifunnel","name":"Mini Funnel"}])))
}
