use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SyncPlanTagRequest {
    pub action: String,
    pub plan_name: String,
    pub source_app: String,
    pub api_key: String,
}

pub async fn sync_plan_tag(
    State(state): State<AppState>,
    Json(req): Json<SyncPlanTagRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let valid_key = std::env::var("INTERNAL_SYNC_KEY")
        .unwrap_or_else(|_| "aca264a01677fdf65e9daae8fa4a0e54bb338d02e7e1660daa3ae6077489aba7".to_string());
    
    if req.api_key != valid_key {
        return Err(AppError::Unauthorized("Invalid API key".into()));
    }
    
    if req.plan_name.trim().is_empty() {
        return Err(AppError::BadRequest("plan_name is required".into()));
    }
    
    // Use the System tenant (first tenant)
    let tenant_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001")
        .map_err(|_| AppError::Internal("Invalid tenant UUID".into()))?;
    
    let group_name = format!("{} Plans", source_app_name(&req.source_app));
    
    // Check if tag group already exists
    let existing_group: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM tag_groups WHERE tenant_id = $1 AND name = $2"
    )
    .bind(tenant_id)
    .bind(&group_name)
    .fetch_optional(&state.pool)
    .await?;
    
    let group_id = if let Some((gid,)) = existing_group {
        gid
    } else {
        let tag_group_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tag_groups (id, tenant_id, name) VALUES ($1, $2, $3)"
        )
        .bind(tag_group_id)
        .bind(tenant_id)
        .bind(&group_name)
        .execute(&state.pool)
        .await?;
        tag_group_id
    };
    
    let tag_name = format!("{}:{}", req.source_app, req.plan_name);
    let tag_color = match req.source_app.as_str() {
        "adaswift" => "#10b981",
        "workflowswift" => "#3b82f6",
        "coreswift" => "#8b5cf6",
        "incentiveswift" => "#f59e0b",
        "missedcall" => "#ef4444",
        _ => "#6366f1"
    };
    
    match req.action.as_str() {
        "create" => {
            let existing_tag: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM tags WHERE tenant_id = $1 AND name = $2"
            )
            .bind(tenant_id)
            .bind(&tag_name)
            .fetch_optional(&state.pool)
            .await?;
            
            if existing_tag.is_some() {
                return Ok(Json(json!({"status": "exists", "message": "Tag already exists"})));
            }
            
            let tag_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO tags (id, tenant_id, group_id, name, color, is_system) VALUES ($1, $2, $3, $4, $5, $6)"
            )
            .bind(tag_id)
            .bind(tenant_id)
            .bind(group_id)
            .bind(&tag_name)
            .bind(tag_color)
            .bind(true)
            .execute(&state.pool)
            .await?;
            
            Ok(Json(json!({
                "status": "created",
                "tag_id": tag_id.to_string(),
                "tag_name": tag_name,
                "tag_color": tag_color,
                "message": format!("System tag '{}' created in FunnelSwift", tag_name)
            })))
        },
        "delete" => {
            let result = sqlx::query(
                "DELETE FROM tags WHERE tenant_id = $1 AND name = $2 AND is_system = true"
            )
            .bind(tenant_id)
            .bind(&tag_name)
            .execute(&state.pool)
            .await?;
            
            if result.rows_affected() > 0 {
                Ok(Json(json!({
                    "status": "deleted",
                    "message": format!("System tag '{}' deleted from FunnelSwift", tag_name)
                })))
            } else {
                Ok(Json(json!({
                    "status": "not_found",
                    "message": format!("No system tag '{}' found to delete", tag_name)
                })))
            }
        },
        _ => Err(AppError::BadRequest(format!("Unknown action: {}. Use 'create' or 'delete'", req.action)))
    }
}

fn source_app_name(slug: &str) -> &str {
    match slug {
        "adaswift" => "ADASwift",
        "workflowswift" => "WorkflowSwift",
        "coreswift" => "CoreSwift",
        "incentiveswift" => "IncentiveSwift",
        "missedcall" => "MissedCall Respondr",
        _ => slug
    }
}
