use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// tag_rules has real columns (name, trigger_tag_id, action_type, action_tag_id, target_app,
/// is_active) but create/update used to answer 201/200 without touching the database, so the
/// API looked healthy while nothing persisted. Both now write, scoped to the caller's tenant.
fn tenant_of(auth: &AuthUser) -> Uuid {
    Uuid::parse_str(&auth.tenant_id).unwrap_or_default()
}

fn opt_uuid(payload: &Value, key: &str) -> Option<Uuid> {
    payload[key].as_str().and_then(|s| Uuid::parse_str(s).ok())
}

pub async fn list_tag_rules(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let rows: Vec<(Uuid, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, COALESCE(name,''), created_at FROM tag_rules WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!(rows)))
}

pub async fn create_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = tenant_of(&auth);
    let name = payload["name"].as_str().unwrap_or("").trim().to_string();
    let trigger_tag_id = opt_uuid(&payload, "trigger_tag_id");
    let action_type = payload["action_type"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let trigger_tag_id =
        trigger_tag_id.ok_or_else(|| AppError::BadRequest("trigger_tag_id is required".into()))?;
    if action_type.is_empty() {
        return Err(AppError::BadRequest("action_type is required".into()));
    }
    let id = Uuid::new_v4();
    let target_app = payload["target_app"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "funnelswift".to_string());
    let is_active = payload["is_active"].as_bool().unwrap_or(true);
    sqlx::query(
        "INSERT INTO tag_rules
           (id, tenant_id, name, description, trigger_tag_id, action_type, action_tag_id, target_app, is_active)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&name)
    .bind(payload["description"].as_str())
    .bind(trigger_tag_id)
    .bind(&action_type)
    .bind(opt_uuid(&payload, "action_tag_id"))
    .bind(&target_app)
    .bind(is_active)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Tag rule created"})),
    ))
}

pub async fn update_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let updated: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE tag_rules SET
           name = COALESCE($3, name),
           description = COALESCE($4, description),
           action_type = COALESCE($5, action_type),
           action_tag_id = COALESCE($6, action_tag_id),
           target_app = COALESCE($7, target_app),
           is_active = COALESCE($8, is_active),
           updated_at = NOW()
         WHERE id = $1 AND tenant_id = $2
         RETURNING id",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(payload["name"].as_str())
    .bind(payload["description"].as_str())
    .bind(payload["action_type"].as_str())
    .bind(opt_uuid(&payload, "action_tag_id"))
    .bind(payload["target_app"].as_str())
    .bind(payload["is_active"].as_bool())
    .fetch_optional(&state.pool)
    .await?;
    if updated.is_none() {
        return Err(AppError::NotFound("Tag rule not found".into()));
    }
    Ok(Json(json!({"id": id, "message": "Tag rule updated"})))
}

pub async fn delete_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    sqlx::query("DELETE FROM tag_rules WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Tag rule deleted"})))
}

pub async fn list_tag_change_log(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!([])))
}
