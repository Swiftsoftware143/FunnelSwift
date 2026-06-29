use axum::{
    extract::State,
    Json,
};
use serde_json::json;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::models::plan_tag_mapping::*;
use crate::state::AppState;

pub async fn list_plan_tag_mappings(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<PlanTagMapping>>> {
    let mappings = sqlx::query_as::<_, PlanTagMapping>("SELECT * FROM plan_tag_mappings ORDER BY created_at")
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(mappings))
}

pub async fn sync_plan_tag_mappings(
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SyncTagMappingRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // Delete existing mappings for this plan
    sqlx::query("DELETE FROM plan_tag_mappings WHERE plan_id = $1")
        .bind(req.plan_id)
        .execute(&state.pool)
        .await?;

    // Insert new mappings
    for tag_id in &req.tag_ids {
        sqlx::query(
            r#"INSERT INTO plan_tag_mappings (plan_id, tag_id, source_software, target_software, commission_rates, allow_dual_routing)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
        )
        .bind(req.plan_id)
        .bind(tag_id)
        .bind(&req.source_software)
        .bind(&req.target_software)
        .bind(&req.commission_rates)
        .bind(req.allow_dual_routing.unwrap_or(false))
        .execute(&state.pool)
        .await?;
    }

    Ok(Json(json!({
        "message": "Plan-tag mappings synced",
        "plan_id": req.plan_id,
        "tag_count": req.tag_ids.len()
    })))
}
