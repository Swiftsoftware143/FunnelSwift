use axum::{
    extract::{Path, Query, State},
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
use crate::security::provider_key_crypto;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct IntegrationTarget {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub portfolio_company_id: Option<Uuid>,
    pub name: String,
    pub webhook_url: String,
    /// Mask of the DECRYPTED credential (first 8 + last 4). The raw key is never in the response:
    /// the column holds `enc:v1:` ciphertext at rest (src/security/provider_key_crypto.rs).
    pub api_key_masked: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateIntegrationTargetRequest {
    pub name: String,
    pub webhook_url: String,
    pub api_key: Option<String>,
    pub portfolio_company_id: Option<Uuid>,
    pub events: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIntegrationTargetRequest {
    pub name: Option<String>,
    pub webhook_url: Option<String>,
    pub api_key: Option<String>,
    pub portfolio_company_id: Option<Uuid>,
    pub events: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct IntegrationTargetQuery {
    pub portfolio_company_id: Option<Uuid>,
}

/// Turn a `target_software` row into the CLIENT-FACING shape: `api_key` is decrypted and masked
/// here, so a caller can never accidentally serialize the stored ciphertext (or the secret).
async fn row_to_integration_target(
    db: &sqlx::PgPool,
    row: &sqlx::postgres::PgRow,
) -> Result<IntegrationTarget, AppError> {
    let stored: Option<String> = row.try_get("api_key").unwrap_or(None);
    let plain = provider_key_crypto::decrypt_optional(db, stored.as_deref()).await?;
    Ok(IntegrationTarget {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        portfolio_company_id: row.try_get("portfolio_company_id")?,
        name: row.try_get("name")?,
        webhook_url: row.try_get("webhook_url")?,
        api_key_masked: plain
            .as_deref()
            .map(provider_key_crypto::mask)
            .unwrap_or_default(),
        events: row.try_get::<Vec<String>, _>("events").unwrap_or_default(),
        is_active: row.try_get("is_active")?,
        // target_software.created_at is TIMESTAMP WITHOUT TIME ZONE: decoding it as
        // DateTime<Utc> failed the whole request the moment a real row existed (the list 500'd and
        // the update path 500'd with it). Read it as NaiveDateTime and publish RFC3339 UTC.
        created_at: row
            .try_get::<chrono::NaiveDateTime, _>("created_at")?
            .and_utc()
            .to_rfc3339(),
    })
}

pub async fn list_integration_targets(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<IntegrationTargetQuery>,
) -> AppResult<Json<Vec<IntegrationTarget>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows = if let Some(company_id) = query.portfolio_company_id {
        sqlx::query(
            "SELECT id, tenant_id, portfolio_company_id, name, webhook_url, api_key, events, is_active, created_at FROM target_software WHERE tenant_id = $1 AND portfolio_company_id = $2 ORDER BY name",
        )
        .bind(tenant_id)
        .bind(company_id)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, tenant_id, portfolio_company_id, name, webhook_url, api_key, events, is_active, created_at FROM target_software WHERE tenant_id = $1 ORDER BY name",
        )
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?
    };

    let mut targets: Vec<IntegrationTarget> = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        targets.push(row_to_integration_target(&state.pool, row).await?);
    }

    Ok(Json(targets))
}

pub async fn create_integration_target(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateIntegrationTargetRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Admin/owner bypass: the tenant admin configuring the Integration Center must always be
    // able to create the core connection (e.g. Mailgun), even on a plan with max_integrations=0.
    // Regular users remain gated by `max_integrations`.
    if !auth.is_admin {
        features::enforce_feature_limit(
            &state,
            tenant_id,
            "max_integrations",
            "Integration targets",
        )
        .await?;
    }
    let target_id = Uuid::new_v4();
    let events = req.events.unwrap_or_default();

    // Encrypt BEFORE the write: the column only ever holds 'enc:v1:' ciphertext, and a missing
    // master key fails the request closed instead of storing the key in the clear.
    let stored_api_key =
        provider_key_crypto::encrypt_optional(&state.pool, req.api_key.as_deref()).await?;

    sqlx::query(
        "INSERT INTO target_software (id, tenant_id, portfolio_company_id, name, webhook_url, api_key, events) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(target_id)
    .bind(tenant_id)
    .bind(req.portfolio_company_id)
    .bind(&req.name)
    .bind(&req.webhook_url)
    .bind(&stored_api_key)
    .bind(&events)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": target_id.to_string(),
            "message": "Integration target created",
            "api_key_masked": req
                .api_key
                .as_deref()
                .map(provider_key_crypto::mask)
                .unwrap_or_default(),
        })),
    ))
}

pub async fn update_integration_target(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateIntegrationTargetRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Fetch existing
    let row = sqlx::query(
        "SELECT id, tenant_id, portfolio_company_id, name, webhook_url, api_key, events, is_active, created_at FROM target_software WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Integration target not found".into()))?;

    let current = row_to_integration_target(&state.pool, &row).await?;
    let current_stored: Option<String> = row.try_get("api_key").unwrap_or(None);

    let name = req.name.unwrap_or(current.name);
    let webhook_url = req.webhook_url.unwrap_or(current.webhook_url);
    // api_key: encrypt what the client submitted, keep the STORED ciphertext when the field was
    // omitted (the SPA sends `null` so a save that does not touch the key cannot replace it), and
    // treat an explicit empty string as "clear". A legacy plaintext row is encrypted on the way
    // through, so the DB guard (migration 051) stays satisfied.
    let (api_key, api_key_masked) = match req.api_key.as_deref() {
        Some(k) if !k.trim().is_empty() => {
            let k = k.trim();
            (
                Some(provider_key_crypto::encrypt_for_storage(&state.pool, k).await?),
                provider_key_crypto::mask(k),
            )
        }
        Some(_) => (Some(String::new()), String::new()),
        None => {
            let stored =
                provider_key_crypto::keep_stored(&state.pool, current_stored.as_deref()).await?;
            let plain =
                provider_key_crypto::decrypt_optional(&state.pool, stored.as_deref()).await?;
            let masked = plain
                .as_deref()
                .map(provider_key_crypto::mask)
                .unwrap_or_default();
            (stored, masked)
        }
    };
    let portfolio_company_id = req.portfolio_company_id.or(current.portfolio_company_id);
    let events = req.events.unwrap_or(current.events);
    let is_active = req.is_active.unwrap_or(current.is_active);

    sqlx::query(
        "UPDATE target_software SET name = $1, webhook_url = $2, api_key = $3, portfolio_company_id = $4, events = $5, is_active = $6 WHERE id = $7 AND tenant_id = $8",
    )
    .bind(&name)
    .bind(&webhook_url)
    .bind(&api_key)
    .bind(portfolio_company_id)
    .bind(&events)
    .bind(is_active)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({
        "message": "Integration target updated",
        "id": id.to_string(),
        "api_key_masked": api_key_masked,
    })))
}

pub async fn delete_integration_target(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("DELETE FROM target_software WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Integration target deleted"})))
}
