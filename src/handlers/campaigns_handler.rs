use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateCampaignRequest {
    pub name: String,
    // The admin console's campaign form posts `type` (www-admin/index.html ->
    // renderCampaigns/openCampaignModal); the column is `campaign_type`. Alias it so the
    // operator's choice is stored instead of silently dropped.
    #[serde(alias = "type")]
    pub campaign_type: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

/// Partial update: an absent field is left as-is (COALESCE in the UPDATE).
/// The console posts {name, type, status}; `type` aliases the `campaign_type` column.
#[derive(Debug, Deserialize)]
pub struct UpdateCampaignRequest {
    pub name: Option<String>,
    #[serde(alias = "type")]
    pub campaign_type: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, FromRow, Serialize)]
pub struct Campaign {
    pub id: String,
    pub tenant_id: Uuid,
    pub name: String,
    pub campaign_type: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<chrono::NaiveDateTime>,
    pub updated_at: Option<chrono::NaiveDateTime>,
}

pub async fn list_campaigns(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let campaigns = sqlx::query_as::<_, Campaign>(
        "SELECT * FROM campaigns WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(serde_json::to_value(campaigns).unwrap_or(json!([]))))
}

pub async fn create_campaign(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCampaignRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let campaign_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO campaigns (id, tenant_id, name, campaign_type, description, status) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&campaign_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.campaign_type)
    .bind(&req.description)
    .bind(&req.status)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": campaign_id, "message": "Campaign created"})),
    ))
}

/// GET /api/v1/campaigns/:id — the console's Edit prefill
/// (www-admin/index.html: openCampaignModal -> rowForEdit(..., api('/campaigns/'+id))).
/// Tenant-scoped: an id owned by another tenant answers 404, never another tenant's row.
pub async fn get_campaign(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let campaign =
        sqlx::query_as::<_, Campaign>("SELECT * FROM campaigns WHERE id = $1 AND tenant_id = $2")
            .bind(&id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Campaign not found".into()))?;

    Ok(Json(serde_json::to_value(campaign).unwrap_or(json!({}))))
}

/// PUT /api/v1/campaigns/:id — the console's Update
/// (openCampaignModal submit -> api('/campaigns/'+id, {method:'PUT'})). Partial update,
/// tenant-scoped; a missing/foreign id answers 404 so the console cannot falsely report success.
pub async fn update_campaign(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCampaignRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let res = sqlx::query(
        "UPDATE campaigns SET \
           name = COALESCE($3, name), \
           campaign_type = COALESCE($4, campaign_type), \
           description = COALESCE($5, description), \
           status = COALESCE($6, status), \
           updated_at = now() \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(&id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.campaign_type)
    .bind(&req.description)
    .bind(&req.status)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("Campaign not found".into()));
    }

    Ok(Json(json!({"id": id, "message": "Campaign updated"})))
}
