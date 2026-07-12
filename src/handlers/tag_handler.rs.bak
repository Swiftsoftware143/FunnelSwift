use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::tag::*;
use crate::state::AppState;
use crate::features;

pub async fn list_tags(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Tag>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let tags = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE tenant_id = $1 OR is_system = true ORDER BY name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(tags))
}

pub async fn create_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_tags", "Tags").await?;
    let tag_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO tags (id, tenant_id, name, color, group_id, metadata) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tag_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.color)
    .bind(req.group_id)
    .bind(&req.metadata)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": tag_id, "message": "Tag created"}))))
}

pub async fn update_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Check if system tag
    let tag = sqlx::query_as::<_, Tag>("SELECT * FROM tags WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Tag not found".into()))?;

    if tag.is_system {
        return Err(AppError::BadRequest("Cannot modify system tags".into()));
    }

    let name = req.name.unwrap_or(tag.name);
    let color = req.color.or(tag.color);
    let group_id = req.group_id.or(tag.group_id);
    let metadata = req.metadata.or(tag.metadata);

    sqlx::query("UPDATE tags SET name=$1, color=$2, group_id=$3, metadata=$4 WHERE id=$5 AND tenant_id=$6")
        .bind(&name)
        .bind(&color)
        .bind(group_id)
        .bind(&metadata)
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Tag updated"})))
}

pub async fn delete_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let tag = sqlx::query_as::<_, Tag>("SELECT * FROM tags WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Tag not found".into()))?;

    if tag.is_system {
        return Err(AppError::BadRequest("Cannot delete system tags".into()));
    }

    sqlx::query("DELETE FROM tags WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Tag deleted"})))
}
