use axum::{extract::{Path, State}, Json};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use crate::auth::middleware::AuthUser;
use crate::error::*;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct TagRule {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub trigger_tag_id: Uuid,
    pub action_type: String,
    pub action_tag_id: Option<Uuid>,
    pub target_app: Option<String>,
    pub is_active: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTagRuleRequest {
    pub name: String,
    pub description: Option<String>,
    pub trigger_tag_id: Uuid,
    pub action_type: String,
    pub action_tag_id: Option<Uuid>,
    pub target_app: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTagRuleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub trigger_tag_id: Option<Uuid>,
    pub action_type: Option<String>,
    pub action_tag_id: Option<Uuid>,
    pub target_app: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TagRuleWithNames {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub trigger_tag_id: Uuid,
    pub trigger_tag_name: Option<String>,
    pub trigger_tag_color: Option<String>,
    pub action_type: String,
    pub action_tag_id: Option<Uuid>,
    pub action_tag_name: Option<String>,
    pub action_tag_color: Option<String>,
    pub target_app: Option<String>,
    pub is_active: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list_tag_rules(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(r#"
        SELECT row_to_json(t.*)::jsonb FROM (
            SELECT 
                r.id, r.tenant_id, r.name, r.description,
                r.trigger_tag_id, tt.name AS trigger_tag_name, tt.color AS trigger_tag_color,
                r.action_type, r.action_tag_id,
                at.name AS action_tag_name, at.color AS action_tag_color,
                r.target_app, r.is_active, r.created_at, r.updated_at
            FROM tag_rules r
            LEFT JOIN tags tt ON tt.id = r.trigger_tag_id
            LEFT JOIN tags at ON at.id = r.action_tag_id
            WHERE r.tenant_id = $1
            ORDER BY r.created_at DESC
        ) t
    "#)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows.into_iter().map(|(v,)| v).collect()))
}

pub async fn create_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTagRuleRequest>,
) -> AppResult<Json<TagRule>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Validate action_type
    if !["add_tag", "remove_tag", "replace"].contains(&req.action_type.as_str()) {
        return Err(AppError::BadRequest("action_type must be add_tag, remove_tag, or replace".into()));
    }

    let rule = sqlx::query_as::<_, TagRule>(
        r#"INSERT INTO tag_rules (tenant_id, name, description, trigger_tag_id, action_type, action_tag_id, target_app, is_active)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.trigger_tag_id)
    .bind(&req.action_type)
    .bind(req.action_tag_id)
    .bind(&req.target_app)
    .bind(req.is_active.unwrap_or(true))
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(rule))
}

pub async fn update_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagRuleRequest>,
) -> AppResult<Json<TagRule>> {
    let tid: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let tid2 = tid; // clone for second use

    let existing = sqlx::query_as::<_, TagRule>(
        "SELECT * FROM tag_rules WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(tid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Tag rule not found".into()))?;

    let name = req.name.unwrap_or(existing.name);
    let description = req.description.or(existing.description);
    let trigger_tag_id = req.trigger_tag_id.unwrap_or(existing.trigger_tag_id);
    let action_type = req.action_type.unwrap_or(existing.action_type);
    let action_tag_id = req.action_tag_id.or(existing.action_tag_id);
    let target_app = req.target_app.or(existing.target_app);
    let is_active = req.is_active.or(existing.is_active);

    let rule = sqlx::query_as::<_, TagRule>(
        r#"UPDATE tag_rules
           SET name = $1, description = $2, trigger_tag_id = $3, action_type = $4,
               action_tag_id = $5, target_app = $6, is_active = $7, updated_at = now()
           WHERE id = $8 AND tenant_id = $9
           RETURNING *"#
    )
    .bind(&name)
    .bind(&description)
    .bind(trigger_tag_id)
    .bind(&action_type)
    .bind(action_tag_id)
    .bind(&target_app)
    .bind(is_active)
    .bind(id)
    .bind(tid2)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(rule))
}


pub async fn list_tag_change_log(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(r#"
        SELECT row_to_json(t.*)::jsonb FROM (
            SELECT cl.*, l.name AS lead_name, l.email AS lead_email
            FROM tag_change_log cl
            LEFT JOIN leads l ON l.id = cl.lead_id
            WHERE cl.tenant_id = $1
            ORDER BY cl.created_at DESC
            LIMIT 50
        ) t
    "#)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows.into_iter().map(|(v,)| v).collect()))
}

pub async fn delete_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tid: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let tid2 = tid; // clone for second use

    let deleted = sqlx::query(
        "DELETE FROM tag_rules WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(tid)
    .execute(&state.pool)
    .await?
    .rows_affected();

    if deleted == 0 {
        return Err(AppError::NotFound("Tag rule not found".into()));
    }

    Ok(Json(json!({"message": "Tag rule deleted"})))
}
