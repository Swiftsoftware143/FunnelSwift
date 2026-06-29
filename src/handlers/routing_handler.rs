use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::models::routing::*;
use crate::state::AppState;

pub async fn list_target_software(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TargetSoftware>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let targets = sqlx::query_as::<_, TargetSoftware>(
        "SELECT * FROM target_software WHERE tenant_id = $1 ORDER BY name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(targets))
}

pub async fn create_target_software(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTargetSoftwareRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_routing_targets", "Routing targets").await?;
    let target_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO target_software (id, tenant_id, name, webhook_url, api_key) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(target_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.webhook_url)
    .bind(&req.api_key)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": target_id, "message": "Target software created"}))))
}

pub async fn list_routing_logs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<RoutingLog>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let logs = sqlx::query_as::<_, RoutingLog>(
        "SELECT * FROM routing_log WHERE source_tenant = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(logs))
}
