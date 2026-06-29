use axum::{extract::State, Json};
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
