use axum::{extract::{Path, State}, Json};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::setting::*;
use crate::state::AppState;

pub async fn get_settings(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TenantSetting>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let settings = sqlx::query_as::<_, TenantSetting>(
        "SELECT * FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(settings))
}

pub async fn update_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateSettingsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query(
        r#"INSERT INTO tenant_settings (tenant_id, key, value) VALUES ($1, $2, $3)
           ON CONFLICT (tenant_id, key) DO UPDATE SET value = $3, updated_at = NOW()"#,
    )
    .bind(tenant_id)
    .bind(&req.key)
    .bind(&req.value)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Setting updated"})))
}
pub async fn delete_setting(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Don't allow deleting seo settings or lead_stages
    if key.starts_with("seo_") || key == "lead_stages" {
        return Err(AppError::BadRequest("Cannot delete protected setting".into()));
    }

    sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1 AND key = $2")
        .bind(tenant_id)
        .bind(&key)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Setting deleted"})))
}