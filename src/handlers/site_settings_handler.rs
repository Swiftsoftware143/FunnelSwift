use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

pub async fn list_site_settings(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, Value)> =
        sqlx::query_as("SELECT id, key, value FROM site_settings ORDER BY key")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn get_site_settings(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<Json<Value>> {
    let row: Option<(Uuid, String, Value)> =
        sqlx::query_as("SELECT id, key, value FROM site_settings WHERE key = $1")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await?;
    let r = row.ok_or_else(|| AppError::NotFound("Settings not found".into()))?;
    Ok(Json(json!({"key": r.1, "value": r.2})))
}
pub async fn update_site_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    sqlx::query("INSERT INTO site_settings (id, key, value) VALUES ($1, $2, $3) ON CONFLICT (key) DO UPDATE SET value = $3")
        .bind(Uuid::new_v4()).bind(&slug).bind(&payload).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Settings updated"})))
}
