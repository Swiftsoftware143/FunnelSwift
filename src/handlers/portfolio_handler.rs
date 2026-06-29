use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct PortfolioCompany {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub settings: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePortfolioCompanyRequest {
    pub name: String,
    pub slug: String,
    pub settings: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePortfolioCompanyRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub settings: Option<Value>,
}

fn row_to_portfolio_company(row: &sqlx::postgres::PgRow) -> Result<PortfolioCompany, sqlx::Error> {
    Ok(PortfolioCompany {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        name: row.try_get("name")?,
        slug: row.try_get("slug")?,
        settings: row.try_get("settings")?,
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")?.to_rfc3339(),
        updated_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")?.to_rfc3339(),
    })
}

pub async fn list_portfolio_companies(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<PortfolioCompany>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows = sqlx::query(
        "SELECT id, tenant_id, name, slug, settings, created_at, updated_at FROM portfolio_companies WHERE tenant_id = $1 ORDER BY name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let companies: Vec<PortfolioCompany> = rows.iter()
        .map(row_to_portfolio_company)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(companies))
}

pub async fn create_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePortfolioCompanyRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_portfolios", "Portfolio companies").await?;
    let company_id = Uuid::new_v4();
    let settings = req.settings.unwrap_or(json!({}));
    let now = chrono::Utc::now();

    sqlx::query(
        "INSERT INTO portfolio_companies (id, tenant_id, name, slug, settings, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(company_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&settings)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": company_id.to_string(), "message": "Portfolio company created"}))))
}

pub async fn get_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<PortfolioCompany>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let row = sqlx::query(
        "SELECT id, tenant_id, name, slug, settings, created_at, updated_at FROM portfolio_companies WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Portfolio company not found".into()))?;

    let company = row_to_portfolio_company(&row)?;

    Ok(Json(company))
}

pub async fn update_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePortfolioCompanyRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Fetch existing
    let row = sqlx::query(
        "SELECT id, tenant_id, name, slug, settings, created_at, updated_at FROM portfolio_companies WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Portfolio company not found".into()))?;

    let current = row_to_portfolio_company(&row)?;
    let now = chrono::Utc::now();

    let name = req.name.unwrap_or(current.name);
    let slug = req.slug.unwrap_or(current.slug);
    let settings = req.settings.unwrap_or(current.settings);

    sqlx::query(
        "UPDATE portfolio_companies SET name = $1, slug = $2, settings = $3, updated_at = $4 WHERE id = $5 AND tenant_id = $6",
    )
    .bind(&name)
    .bind(&slug)
    .bind(&settings)
    .bind(now)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Portfolio company updated", "id": id.to_string()})))
}

pub async fn delete_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("DELETE FROM portfolio_companies WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Portfolio company deleted"})))
}
