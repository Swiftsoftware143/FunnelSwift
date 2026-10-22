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

/// Column widths the campaign writers bind into, read from the live schema: `campaigns.name`
/// `varchar(255)`, `campaign_type` `varchar(64)`, `status` `varchar(32)` (`description` is `text`).
/// Neither writer checked any of them, so an over-long name posted from the panel reached the
/// INSERT/UPDATE and came back as the raw `500 {"error":"Database error"}` — the same class the
/// tag-name fix closed on `tags`.
const CAMPAIGN_NAME_CHARS: usize = 255;
const CAMPAIGN_TYPE_CHARS: usize = 64;
const CAMPAIGN_STATUS_CHARS: usize = 32;

/// Refuse a value the column cannot store, naming the field the way the panel labels it.
fn check_storable(value: &str, max_chars: usize, label: &str) -> AppResult<()> {
    crate::security::display_name::check_len(value, max_chars, label)
        .map_err(AppError::UnprocessableEntity)
}

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

    // Refused before the write, so the console shows the field instead of a server error.
    check_storable(&req.name, CAMPAIGN_NAME_CHARS, "name")?;
    if let Some(t) = req.campaign_type.as_deref() {
        check_storable(t, CAMPAIGN_TYPE_CHARS, "type")?;
    }
    if let Some(s) = req.status.as_deref() {
        check_storable(s, CAMPAIGN_STATUS_CHARS, "status")?;
    }

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

    // A rename or a status/type change carries the risk on the UPDATE exactly as the create does.
    if let Some(n) = req.name.as_deref() {
        check_storable(n, CAMPAIGN_NAME_CHARS, "name")?;
    }
    if let Some(t) = req.campaign_type.as_deref() {
        check_storable(t, CAMPAIGN_TYPE_CHARS, "type")?;
    }
    if let Some(s) = req.status.as_deref() {
        check_storable(s, CAMPAIGN_STATUS_CHARS, "status")?;
    }

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

/// DELETE /api/v1/campaigns/:id — remove one campaign this workspace created.
///
/// The campaigns screen shipped with three of the four CRUD arms and no DELETE, so a campaign
/// created by mistake (a typo, a duplicate, a trial run) could be renamed and re-statused but never
/// taken off the list — permanent for the life of the workspace. Every other list the console
/// offers (leads, tags, funnels) can be pruned, so this was the one screen where a mistake was
/// forever. Measured live 2026-10-22 with a probe workspace on the served console: `DELETE
/// /api/v1/campaigns/<id>` answered `405 Method Not Allowed`, and the row was still returned by
/// `GET /api/v1/campaigns` afterwards.
///
/// Tenant-scoped exactly like its siblings: the id is matched WITH `tenant_id`, so a campaign
/// belonging to another workspace answers `404` — it is neither deleted nor even confirmed to
/// exist. `campaigns` has no inbound foreign keys (measured: `pg_constraint` `contype='f'` +
/// `confrelid='campaigns'::regclass` = 0 rows), so this is a single-row delete with no children to
/// reconsider.
pub async fn delete_campaign(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let res = sqlx::query("DELETE FROM campaigns WHERE id = $1 AND tenant_id = $2")
        .bind(&id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("Campaign not found".into()));
    }

    Ok(Json(json!({"id": id, "message": "Campaign deleted"})))
}
