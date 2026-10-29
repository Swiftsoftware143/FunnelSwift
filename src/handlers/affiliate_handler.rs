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

/// Stored form of an affiliate's address, or a sentence naming the field.
///
/// `affiliates.email` is not a detail: it is the KEY this person is found by. The portal resolves
/// the signed-in user's affiliate row with `WHERE email = $1` (`affiliate_portal_handler`), and a
/// commission is credited by matching the referring customer's login against it
/// (`affiliate_tracking_handler`). So a value that is not an address, or one stored with capitals
/// no lookup will ever produce, is an affiliate who can never sign in to their portal and can never
/// be credited. Measured live 2026-10-29 before this guard: `POST /api/v1/affiliates` with
/// `{"name":"…","email":"not-an-email"}` answered **201** and stored the row, and the console's own
/// `<input type=email>` is the only thing that ever refused it — an API caller was not refused at
/// all. Same class and same module as the leads door (`email_addr::check_lead_address`, 2026-10-29).
///
/// An EMPTY address stays legal: the console's Email field is optional, and an affiliate record with
/// no address is a separate (visible) state, not a typo. Anything non-empty is trimmed and
/// lowercased, because every read of this column is an exact match.
fn affiliate_email_for_storage(raw: &str) -> AppResult<String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    crate::security::email_addr::normalize(raw).map_err(AppError::UnprocessableEntity)
}

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
///
/// A MACRO, not a `const`, since the widened gate rule 5d (2026-10-02) reports a statement BUILT at
/// run time: `format!("SELECT {} FROM …", COLS)` assembles SQL in the binary. The sanctioned shape
/// (gate-pre-build.sh rule 5d, and what WorkflowSwift's handlers already do) is compile-time
/// assembly — `cols!()` expands to a string LITERAL, so every `concat!` below is ONE literal to the
/// compiler and `concat!` is blind to it, deliberately. The SQL text is byte-identical to what the
/// `format!` produced.
macro_rules! affiliate_cols {
    () => {
        "id, tenant_id, name, email, industry, \
    commission_rate::float8 AS commission_rate, tax_docs, is_active, is_visible, tags, \
    created_at, updated_at, override_commission_rate::float8 AS override_commission_rate, \
    override_note, rate_reason, rate_band_id, rate_updated_at"
    };
}

/// Same treatment for `affiliate_commissions.amount` NUMERIC(10,2) vs
/// `AffiliateCommission::amount: f64`.
macro_rules! affiliate_commission_cols {
    () => {
        "id, affiliate_id, lead_id, \
    amount::float8 AS amount, status, paid_at, created_at"
    };
}

/// The three statements these columns are read by, assembled at COMPILE time (see `affiliate_cols!`).
const SQL_LIST_AFFILIATES: &str = concat!(
    "SELECT ",
    affiliate_cols!(),
    " FROM affiliates WHERE tenant_id = $1 ORDER BY created_at DESC"
);

const SQL_AFFILIATE_BY_ID: &str = concat!(
    "SELECT ",
    affiliate_cols!(),
    " FROM affiliates WHERE id = $1 AND tenant_id = $2"
);

const SQL_COMMISSIONS_BY_AFFILIATE: &str = concat!(
    "SELECT ",
    affiliate_commission_cols!(),
    " FROM affiliate_commissions WHERE affiliate_id = $1 ORDER BY created_at DESC"
);

pub async fn list_affiliates(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Affiliate>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let affiliates = sqlx::query_as::<_, Affiliate>(SQL_LIST_AFFILIATES)
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

    // Refuse a value that is not an address BEFORE the duplicate check, so the caller is told
    // about the typo itself instead of being told a malformed string "already exists".
    let email = affiliate_email_for_storage(&req.email)?;

    // Check for duplicate email within tenant
    if !email.is_empty() {
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM affiliates WHERE email = $1 AND tenant_id = $2)",
        )
        .bind(&email)
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(false);

        if existing {
            return Err(AppError::BadRequest(format!(
                "An affiliate with email '{}' already exists in this workspace",
                email
            )));
        }
    }

    let aff_id = generate_affiliate_id();

    sqlx::query(
        "INSERT INTO affiliates (id, tenant_id, name, email, industry, commission_rate, override_commission_rate, override_note, tax_docs, tags, is_visible) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&aff_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&email)
    .bind(&req.industry)
    .bind(req.commission_rate)
    .bind(req.override_commission_rate)
    .bind(&req.override_note)
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

    let affiliate = sqlx::query_as::<_, Affiliate>(SQL_AFFILIATE_BY_ID)
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

    let existing = sqlx::query_as::<_, Affiliate>(SQL_AFFILIATE_BY_ID)
        .bind(&id)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    // The same rule at the edit door: a typo, or capitals the portal lookup can never produce,
    // is the same dead affiliate. An ABSENT email still means "leave the stored one alone".
    let email_val = match req.email.as_deref() {
        Some(raw) => affiliate_email_for_storage(raw)?,
        None => existing.email.clone(),
    };

    let tags_val = req.tags.or(existing.tags);
    let visible_val = req.is_visible.or(existing.is_visible);

    let result = sqlx::query(
        "UPDATE affiliates SET name=$1, email=$2, industry=$3, commission_rate=$4, tax_docs=$5, is_active=$6, tags=$7, is_visible=$8, override_commission_rate=CASE WHEN $9 THEN NULL ELSE COALESCE($10, override_commission_rate) END, override_note=CASE WHEN $9 THEN NULL ELSE COALESCE($11, override_note) END, updated_at=NOW() WHERE id=$12 AND tenant_id=$13",
    )
    .bind(req.name.unwrap_or(existing.name))
    .bind(&email_val)
    .bind(req.industry.or(existing.industry))
    .bind(req.commission_rate.or(existing.commission_rate))
    .bind(req.tax_docs.or(existing.tax_docs))
    .bind(req.is_active.unwrap_or(existing.is_active))
    .bind(tags_val)
    .bind(visible_val)
    .bind(req.clear_override.unwrap_or(false))
    .bind(req.override_commission_rate)
    .bind(&req.override_note)
    .bind(&id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Affiliate not found".into()));
    }

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
    let _ = sqlx::query_as::<_, Affiliate>(SQL_AFFILIATE_BY_ID)
        .bind(&id)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;

    let commissions = sqlx::query_as::<_, AffiliateCommission>(SQL_COMMISSIONS_BY_AFFILIATE)
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

    let _existing = sqlx::query_as::<_, Affiliate>(SQL_AFFILIATE_BY_ID)
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
