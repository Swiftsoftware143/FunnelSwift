use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn list_tag_rules(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, COALESCE(name,''), created_at FROM tag_rules ORDER BY created_at DESC"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn create_tag_rule(_auth: AuthUser, State(_state): State<AppState>, Json(_payload): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(json!({"message": "Tag rule created"}))))
}
pub async fn update_tag_rule(_auth: AuthUser, State(_state): State<AppState>, Path(_id): Path<Uuid>, Json(_payload): Json<Value>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "Tag rule updated"})))
}
pub async fn delete_tag_rule(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM tag_rules WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Tag rule deleted"})))
}
pub async fn list_tag_change_log(_auth: AuthUser, State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([])))
}
