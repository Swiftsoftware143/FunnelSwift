use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::models::affiliate::*;
use crate::state::AppState;

fn generate_affiliate_id() -> String {
    let now = chrono::Utc::now();
    let date_part = now.format("%m%d%Y").to_string();
    let random_part: String = (0..5)
        .map(|_| {
            let n = rand::random::<u8>() % 36;
            if n < 10 {
                (b'0' + n) as char
            } else {
                (b'A' + n - 10) as char
            }
        })
        .collect();
    format!("AFF-{}-{}", date_part, random_part)
}

/// Explicit column list for `affiliates`.
///
/// `SELECT *` dies at runtime the moment a real row is decoded: `commission_rate` is NUMERIC in
/// the live DB while `Affiliate::commission_rate` is `Option<f64>`, and sqlx-postgres maps f64 to
/// FLOAT8 only (it has no `impl Type<Postgres>` for numeric) -> `GET /affiliates` answered HTTP 500
/// "Database error" with `mismatched types ... `NUMERIC`` (kanban t_4385aa2c). Casting keeps the
/// struct unchanged, exactly like `PLAN_COLS` in plan_handler.rs.
pub const AFFILIATE_COLS: &str = "id, tenant_id, name, email, industry, \
    commission_rate::float8 AS commission_rate, tax_docs, is_active, is_visible, tags, \
    created_at, updated_at";

/// Same treatment for `affiliate_commissions.amount` NUMERIC(10,2) vs
/// `AffiliateCommission::amount: f64`.
pub const AFFILIATE_COMMISSION_COLS: &str = "id, affiliate_id, lead_id, \
    amount::float8 AS amount, status, paid_at, created_at";

pub async fn list_affiliates(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Affiliate>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let affiliates = sqlx::query_as::<_, Affiliate>(&format!(
        "SELECT {} FROM affiliates WHERE tenant_id = $1 ORDER BY created_at DESC",
        AFFILIATE_COLS
    ))
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(affiliates))
}

pub async fn create_affiliate(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAffiliateRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_affiliates", "Affiliates").await?;

    // Check for duplicate email within tenant
    if !req.email.trim().is_empty() {
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM affiliates WHERE email = $1 AND tenant_id = $2)",
        )
        .bind(&req.email)
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(false);

        if existing {
            return Err(AppError::BadRequest(format!(
                "An affiliate with email '{}' already exists in this workspace",
                req.email
            )));
        }
    }

    let aff_id = generate_affiliate_id();

    sqlx::query(
        "INSERT INTO affiliates (id, tenant_id, name, email, industry, commission_rate, tax_docs, tags, is_visible) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&aff_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.email)
    .bind(&req.industry)
    .bind(req.commission_rate)
    .bind(&req.tax_docs)
    .bind(&req.tags)
    .bind(req.is_visible)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": aff_id, "message": "Affiliate created"})),
    ))
}

pub async fn get_affiliate(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Affiliate>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let affiliate = sqlx::query_as::<_, Affiliate>(&format!(
        "SELECT {} FROM affiliates WHERE id = $1 AND tenant_id = $2",
        AFFILIATE_COLS
    ))
    .bind(&id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    Ok(Json(affiliate))
}

pub async fn update_affiliate(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAffiliateRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_as::<_, Affiliate>(&format!(
        "SELECT {} FROM affiliates WHERE id = $1 AND tenant_id = $2",
        AFFILIATE_COLS
    ))
    .bind(&id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    let tags_val = req.tags.or(existing.tags);
    let visible_val = req.is_visible.or(existing.is_visible);

    sqlx::query(
        "UPDATE affiliates SET name=$1, email=$2, industry=$3, commission_rate=$4, tax_docs=$5, is_active=$6, tags=$7, is_visible=$8, updated_at=NOW() WHERE id=$9 AND tenant_id=$10",
    )
    .bind(req.name.unwrap_or(existing.name))
    .bind(req.email.unwrap_or(existing.email))
    .bind(req.industry.or(existing.industry))
    .bind(req.commission_rate.or(existing.commission_rate))
    .bind(req.tax_docs.or(existing.tax_docs))
    .bind(req.is_active.unwrap_or(existing.is_active))
    .bind(tags_val)
    .bind(visible_val)
    .bind(&id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Affiliate updated"})))
}

pub async fn get_affiliate_commissions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<AffiliateCommission>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Verify affiliate exists and belongs to tenant
    let _ = sqlx::query_as::<_, Affiliate>(&format!(
        "SELECT {} FROM affiliates WHERE id = $1 AND tenant_id = $2",
        AFFILIATE_COLS
    ))
    .bind(&id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    let commissions = sqlx::query_as::<_, AffiliateCommission>(&format!(
        "SELECT {} FROM affiliate_commissions WHERE affiliate_id = $1 ORDER BY created_at DESC",
        AFFILIATE_COMMISSION_COLS
    ))
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(commissions))
}
pub async fn delete_affiliate(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let _existing = sqlx::query_as::<_, Affiliate>(&format!(
        "SELECT {} FROM affiliates WHERE id = $1 AND tenant_id = $2",
        AFFILIATE_COLS
    ))
    .bind(&id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    sqlx::query("DELETE FROM affiliates WHERE id = $1 AND tenant_id = $2")
        .bind(&id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Affiliate deleted"})))
}
