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
    // The contact half of the row. The columns have existed since the table shipped and POST
    // already writes them, but every read omitted them, so the console's Company -> Email column
    // was permanently "-" and the description never reached the screen at all.
    pub email: Option<String>,
    pub description: Option<String>,
    pub settings: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePortfolioCompanyRequest {
    pub name: String,
    pub slug: String,
    pub email: Option<String>,
    pub description: Option<String>,
    pub settings: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePortfolioCompanyRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub email: Option<String>,
    pub description: Option<String>,
    pub settings: Option<Value>,
}

fn row_to_portfolio_company(row: &sqlx::postgres::PgRow) -> Result<PortfolioCompany, sqlx::Error> {
    Ok(PortfolioCompany {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        name: row.try_get("name")?,
        slug: row.try_get("slug")?,
        email: row.try_get("email")?,
        description: row.try_get("description")?,
        settings: row.try_get("settings")?,
        created_at: row
            .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")?
            .to_rfc3339(),
        updated_at: row
            .try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")?
            .to_rfc3339(),
    })
}

pub async fn list_portfolio_companies(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<PortfolioCompany>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows = sqlx::query(
        "SELECT id, tenant_id, name, slug, email, description, settings, created_at, updated_at FROM portfolio_companies WHERE tenant_id = $1 ORDER BY name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let companies: Vec<PortfolioCompany> = rows
        .iter()
        .map(row_to_portfolio_company)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(companies))
}

pub async fn create_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePortfolioCompanyRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_portfolios", "Portfolio companies")
        .await?;
    // The address on a portfolio company is a CONTACT this install mails and the value
    // portfolio-sync copies into another app's `tenants.email`, so a string that is not an address
    // is a permanently unreachable contact. Absent (or empty) stays legal, exactly as on a lead.
    if let Some(raw) = req.email.as_deref() {
        crate::security::email_addr::check_lead_address(raw)
            .map_err(AppError::UnprocessableEntity)?;
    }
    let company_id = Uuid::new_v4();
    let settings = req.settings.unwrap_or(json!({}));
    let now = chrono::Utc::now();

    sqlx::query(
        "INSERT INTO portfolio_companies (id, tenant_id, name, slug, email, description, settings, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(company_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.email)
    .bind(&req.description)
    .bind(&settings)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": company_id.to_string(), "message": "Portfolio company created"})),
    ))
}

pub async fn get_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<PortfolioCompany>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let row = sqlx::query(
        "SELECT id, tenant_id, name, slug, email, description, settings, created_at, updated_at FROM portfolio_companies WHERE id = $1 AND tenant_id = $2",
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
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Fetch existing
    let row = sqlx::query(
        "SELECT id, tenant_id, name, slug, email, description, settings, created_at, updated_at FROM portfolio_companies WHERE id = $1 AND tenant_id = $2",
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
    // Contact fields are preserved when the caller omits them, exactly like name/slug: the edit
    // form always sends all four, so an absent field means "not part of this edit", not "clear it".
    let email = req.email.or(current.email);
    if let Some(raw) = email.as_deref() {
        crate::security::email_addr::check_lead_address(raw)
            .map_err(AppError::UnprocessableEntity)?;
    }
    let description = req.description.or(current.description);

    sqlx::query(
        "UPDATE portfolio_companies SET name = $1, slug = $2, settings = $3, email = $4, description = $5, updated_at = $6 WHERE id = $7 AND tenant_id = $8",
    )
    .bind(&name)
    .bind(&slug)
    .bind(&settings)
    .bind(&email)
    .bind(&description)
    .bind(now)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(
        json!({"message": "Portfolio company updated", "id": id.to_string()}),
    ))
}

pub async fn delete_portfolio_company(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let result = sqlx::query("DELETE FROM portfolio_companies WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Portfolio company not found".into()));
    }

    Ok(Json(json!({"message": "Portfolio company deleted"})))
}
