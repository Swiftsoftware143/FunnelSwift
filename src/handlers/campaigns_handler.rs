use axum::{extract::State, http::StatusCode, Json};
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
