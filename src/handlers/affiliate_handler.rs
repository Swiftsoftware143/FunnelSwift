use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::affiliate::*;
use crate::state::AppState;
use crate::features;

fn generate_affiliate_id() -> String {
    let now = chrono::Utc::now();
    let date_part = now.format("%m%d%Y").to_string();
    let random_part: String = (0..5).map(|_| {
        let n = rand::random::<u8>() % 36;
        if n < 10 { (b'0' + n) as char } else { (b'A' + n - 10) as char }
    }).collect();
    format!("AFF-{}-{}", date_part, random_part)
}

pub async fn list_affiliates(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Affiliate>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let affiliates = sqlx::query_as::<_, Affiliate>(
        "SELECT * FROM affiliates WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
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
            "An affiliate with email '{}' already exists in this workspace", req.email
        )));
    }
}

    let aff_id = generate_affiliate_id();

    sqlx::query(
        "INSERT INTO affiliates (id, tenant_id, name, email, industry, commission_rate, tax_docs) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&aff_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.email)
    .bind(&req.industry)
    .bind(req.commission_rate)
    .bind(&req.tax_docs)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": aff_id, "message": "Affiliate created"}))))
}

pub async fn get_affiliate(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Affiliate>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let affiliate = sqlx::query_as::<_, Affiliate>(
        "SELECT * FROM affiliates WHERE id = $1 AND tenant_id = $2",
    )
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_as::<_, Affiliate>(
        "SELECT * FROM affiliates WHERE id = $1 AND tenant_id = $2",
    )
    .bind(&id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    sqlx::query(
        "UPDATE affiliates SET name=$1, email=$2, industry=$3, commission_rate=$4, tax_docs=$5, is_active=$6, updated_at=NOW() WHERE id=$7 AND tenant_id=$8",
    )
    .bind(req.name.unwrap_or(existing.name))
    .bind(req.email.unwrap_or(existing.email))
    .bind(req.industry.or(existing.industry))
    .bind(req.commission_rate.or(existing.commission_rate))
    .bind(req.tax_docs.or(existing.tax_docs))
    .bind(req.is_active.unwrap_or(existing.is_active))
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Verify affiliate exists and belongs to tenant
    let _ = sqlx::query_as::<_, Affiliate>(
        "SELECT * FROM affiliates WHERE id = $1 AND tenant_id = $2",
    )
    .bind(&id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    let commissions = sqlx::query_as::<_, AffiliateCommission>(
        "SELECT * FROM affiliate_commissions WHERE affiliate_id = $1 ORDER BY created_at DESC",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(commissions))
}
