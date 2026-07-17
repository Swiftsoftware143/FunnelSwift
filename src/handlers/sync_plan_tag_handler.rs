use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const SYSTEM_TENANT: &str = "00000000-0000-0000-0000-000000000001";
const ADMIN_CORESWIFT_TENANT: &str = "abd8ad22-aa01-4642-9a9f-6bef6a03d85b";

#[derive(Debug, Deserialize)]
pub struct SyncPlanTagRequest {
    pub action: String,
    pub plan_name: String,
    pub source_app: String,
    pub api_key: String,
    /// "admin", "portfolio_company", or "user"
    #[serde(default = "default_entity")]
    pub source_entity: String,
    /// Portfolio company slug (required when source_entity = "portfolio_company")
    pub entity_slug: Option<String>,
    /// Caller's tenant ID in the source app (required for user-level sync)
    pub source_tenant_id: Option<String>,
}

fn default_entity() -> String {
    "admin".to_string()
}

/// Resolve the CoreSwift target tenant_id and optional portfolio company slug.
fn resolve_coreswift_target(
    entity: &str,
    slug: &Option<String>,
    caller_tenant: &Option<String>,
) -> (String, Option<String>) {
    match entity {
        "admin" => (ADMIN_CORESWIFT_TENANT.to_string(), None),
        "portfolio_company" => {
            // Portfolio company tags go to the admin tenant in CoreSwift
            // but we pass the slug so the receiver can scope to the right company
            (ADMIN_CORESWIFT_TENANT.to_string(), slug.clone())
        }
        "user" => {
            let tid = caller_tenant.clone().unwrap_or_default();
            if tid.is_empty() {
                (ADMIN_CORESWIFT_TENANT.to_string(), None)
            } else {
                (tid, None)
            }
        }
        _ => (ADMIN_CORESWIFT_TENANT.to_string(), None),
    }
}

pub async fn sync_plan_tag(
    State(state): State<AppState>,
    Json(req): Json<SyncPlanTagRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let valid_key = std::env::var("INTERNAL_SYNC_KEY")
        .map_err(|_| AppError::Internal("INTERNAL_SYNC_KEY not configured".into()))?;
    
    if req.api_key != valid_key {
        return Err(AppError::Unauthorized("Invalid API key".into()));
    }
    
    if req.plan_name.trim().is_empty() {
        return Err(AppError::BadRequest("plan_name is required".into()));
    }

    // Tags in FunnelSwift always live under the System tenant
    let system_tenant = Uuid::parse_str(SYSTEM_TENANT)
        .map_err(|_| AppError::Internal("Invalid system tenant UUID".into()))?;
    
    let group_name = format!("{} Plans", source_app_name(&req.source_app));
    
    // Check if tag group already exists
    let existing_group: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM tag_groups WHERE tenant_id = $1 AND name = $2"
    )
    .bind(system_tenant)
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
        .bind(system_tenant)
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

    // Prepare CoreSwift sync params (resolve tenant target based on entity context)
    let cs_url = std::env::var("CORESWIFT_URL").unwrap_or_else(|_| "http://localhost:8084".to_string());
    let cs_key = std::env::var("INTERNAL_SYNC_KEY").map_err(|_| AppError::Internal("INTERNAL_SYNC_KEY not configured".into()))?;
    let (cs_tenant_id, cs_company_slug) = resolve_coreswift_target(
        &req.source_entity,
        &req.entity_slug,
        &req.source_tenant_id,
    );
    
    let plan_name = req.plan_name.clone();
    let tag_color_clone = tag_color.to_string();
    
    match req.action.as_str() {
        "create" => {
            let existing_tag: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM tags WHERE tenant_id = $1 AND name = $2"
            )
            .bind(system_tenant)
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
            .bind(system_tenant)
            .bind(group_id)
            .bind(&tag_name)
            .bind(&tag_color)
            .bind(true)
            .execute(&state.pool)
            .await?;
            
            // Sync to CoreSwift (fire-and-forget) — routed by entity context
            let cs = cs_url.clone();
            let ck = cs_key.clone();
            let ct = cs_tenant_id.clone();
            let cslug = cs_company_slug.clone();
            let pn = plan_name.clone();
            let tn = tag_name.clone();
            let tc = tag_color_clone.clone();
            tokio::spawn(async move {
                let mut payload = serde_json::json!({
                    "tenant_id": ct,
                    "name": &pn,
                    "color": tc,
                });
                if let Some(slug) = cslug {
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert("company_slug".to_string(), serde_json::json!(slug));
                    }
                }
                let url = format!("{}/api/internal/tags", cs.trim_end_matches('/'));
                match reqwest::Client::new()
                    .post(&url)
                    .header("x-internal-key", &ck)
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if !status.is_success() && status.as_u16() != 409 {
                            let body = resp.text().await.unwrap_or_default();
                            tracing::warn!("sync-plan-tag: CoreSwift create '{}' failed: {} - {}", pn, status, body);
                        }
                    }
                    Err(e) => tracing::warn!("sync-plan-tag: CoreSwift create '{}' error: {}", pn, e),
                }
            });

            Ok(Json(json!({
                "status": "created",
                "tag_id": tag_id.to_string(),
                "tag_name": tag_name,
                "tag_color": tag_color,
                "message": format!("Tag '{}' created in FunnelSwift", tag_name)
            })))
        },
        "delete" => {
            let result = sqlx::query(
                "DELETE FROM tags WHERE tenant_id = $1 AND name = $2 AND is_system = true"
            )
            .bind(system_tenant)
            .bind(&tag_name)
            .execute(&state.pool)
            .await?;
            
            if result.rows_affected() > 0 {
                Ok(Json(json!({
                    "status": "deleted",
                    "message": format!("Tag '{}' deleted from FunnelSwift", tag_name)
                })))
            } else {
                Ok(Json(json!({
                    "status": "not_found",
                    "message": format!("No tag '{}' found to delete", tag_name)
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
