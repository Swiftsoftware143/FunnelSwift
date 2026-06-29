use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::models::tag_group::*;
use crate::state::AppState;

pub async fn list_tag_groups(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TagGroup>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let groups = sqlx::query_as::<_, TagGroup>(
        "SELECT * FROM tag_groups WHERE tenant_id = $1 ORDER BY sort_order, name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(groups))
}

pub async fn create_tag_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTagGroupRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_tag_groups", "Tag groups").await?;
    let group_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO tag_groups (id, tenant_id, name, is_collapsible, sort_order) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(req.is_collapsible.unwrap_or(true))
    .bind(req.sort_order.unwrap_or(0))
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": group_id, "message": "Tag group created"}))))
}

pub async fn update_tag_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagGroupRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_as::<_, TagGroup>(
        "SELECT * FROM tag_groups WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tag group not found".into()))?;

    let name = req.name.unwrap_or(existing.name);
    let is_collapsible = req.is_collapsible.unwrap_or(existing.is_collapsible);
    let sort_order = req.sort_order.unwrap_or(existing.sort_order);

    sqlx::query("UPDATE tag_groups SET name=$1, is_collapsible=$2, sort_order=$3 WHERE id=$4 AND tenant_id=$5")
        .bind(&name)
        .bind(is_collapsible)
        .bind(sort_order)
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Tag group updated"})))
}

pub async fn delete_tag_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Clear group_id from tags first
    sqlx::query("UPDATE tags SET group_id = NULL WHERE group_id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    sqlx::query("DELETE FROM tag_groups WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Tag group deleted"})))
}
