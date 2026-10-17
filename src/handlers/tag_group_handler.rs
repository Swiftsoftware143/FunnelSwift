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

/// `tag_groups.name` is `varchar(255)` and neither writer checked it, so an over-long group name
/// typed into the panel reached the INSERT/UPDATE and came back as the raw
/// `500 {"error":"Database error"}`. Measured live 2026-10-17: `POST /api/v1/tag-groups` with a
/// 300-character name answered 500, and the same value on a rename hit the UPDATE the same way.
/// Call it BEFORE the write (same boundary the tag-name fix uses) so the console gets a 422 that
/// names the field, and bind the RETURNED value.
const GROUP_NAME_CHARS: usize = 255;

fn clean_group_name(raw: &str) -> AppResult<String> {
    crate::security::display_name::normalize_capped(raw, GROUP_NAME_CHARS, "name")
        .map_err(AppError::UnprocessableEntity)
}

pub async fn list_tag_groups(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TagGroup>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let groups = sqlx::query_as::<_, TagGroup>(
        "SELECT * FROM tag_groups WHERE tenant_id = $1 ORDER BY sort_order, name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(groups))
}

/// The admin console's tag-group VOCABULARY (kanban t_968c2b09).
///
/// The fleet's shared taxonomy lives on the system tenant (`src/system_tenant.rs`): its tags are
/// `is_system = true` and their `tag_groups` rows belong to that tenant, so the tenant-scoped
/// `list_tag_groups` answers `[]` for the operator's own tenant and the console could not name a
/// single taxonomy group -- it could only print the raw `a0000000-...` id.
///
/// This is an admin-only READ of the vocabulary the console has to NAME: the caller's own groups,
/// the system tenant's shared groups (`src/system_tenant.rs` — the same `tenant_id = $1 OR
/// tenant_id = $n` predicate the other admin-facing listings use), and any group a visible tag
/// actually points at — so a Group cell can always be resolved even when a tag references a group
/// owned by some third tenant. It is a read: renaming and deleting stay on the tenant-scoped routes,
/// and the console offers those controls only for its own rows.
///
/// Measured while choosing the predicate: the groups referenced by the visible tags alone are FIVE
/// of the six (`Custom` has no tag pointing at it), so a referenced-only read would hide a real
/// group of the shared vocabulary from the very screen that lists that vocabulary. The shared route
/// is deliberately NOT widened: `update_tag_group`/`delete_tag_group` are `WHERE tenant_id = $1`, so
/// every tenant shell would gain rows it can neither rename nor delete.
pub async fn list_tag_groups_admin(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<TagGroup>>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let groups = sqlx::query_as::<_, TagGroup>(
        "SELECT * FROM tag_groups \
         WHERE tenant_id = $1 \
            OR tenant_id = $2 \
            OR id IN (SELECT group_id FROM tags \
                       WHERE (tenant_id = $1 OR is_system = true) AND group_id IS NOT NULL) \
         ORDER BY sort_order, name",
    )
    .bind(tenant_id)
    .bind(crate::system_tenant::system_tenant_id())
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(groups))
}

pub async fn create_tag_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTagGroupRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let name = clean_group_name(&req.name)?;
    features::enforce_feature_limit(&state, tenant_id, "max_tag_groups", "Tag groups").await?;
    let group_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO tag_groups (id, tenant_id, name, is_collapsible, sort_order) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(req.is_collapsible.unwrap_or(true))
    .bind(req.sort_order.unwrap_or(0))
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": group_id, "message": "Tag group created"})),
    ))
}

pub async fn update_tag_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagGroupRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing =
        sqlx::query_as::<_, TagGroup>("SELECT * FROM tag_groups WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Tag group not found".into()))?;

    // A rename is validated before the UPDATE for the same reason the create is: the column
    // cannot hold it, so it must be refused with a readable reason rather than a database error.
    let name = match req.name {
        Some(raw) => clean_group_name(&raw)?,
        None => existing.name,
    };
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
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Clear group_id from tags first
    sqlx::query("UPDATE tags SET group_id = NULL WHERE group_id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    let result = sqlx::query("DELETE FROM tag_groups WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Tag group not found".into()));
    }

    Ok(Json(json!({"message": "Tag group deleted"})))
}
