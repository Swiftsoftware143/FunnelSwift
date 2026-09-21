use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::models::routing::*;
use crate::security::provider_key_crypto;
use crate::state::AppState;

pub async fn list_target_software(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let targets = sqlx::query_as::<_, TargetSoftware>(
        "SELECT id, tenant_id, name, webhook_url, api_key, portfolio_company_id, events, is_active, created_at \
         FROM target_software WHERE tenant_id = $1 ORDER BY name",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    // target_software.api_key is CIPHERTEXT at rest. Decrypt it and publish a mask built from the
    // DECRYPTED value: neither the secret nor the ciphertext ever reaches a client (t_63840ff2).
    let mut out: Vec<Value> = Vec::with_capacity(targets.len());
    for t in targets {
        let plain =
            provider_key_crypto::decrypt_optional(&state.pool, t.stored_api_key.as_deref()).await?;
        out.push(json!({
            "id": t.id,
            "tenant_id": t.tenant_id,
            "name": t.name,
            "webhook_url": t.webhook_url,
            "api_key_masked": plain.as_deref().map(provider_key_crypto::mask).unwrap_or_default(),
            "portfolio_company_id": t.portfolio_company_id,
            "events": t.events,
            "is_active": t.is_active,
            "created_at": t.created_at,
        }));
    }

    Ok(Json(json!(out)))
}

pub async fn create_target_software(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTargetSoftwareRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_flag(&state, tenant_id, "has_dual_routing", "Dual routing").await?;
    features::enforce_feature_limit(&state, tenant_id, "max_routing_targets", "Routing targets")
        .await?;
    let target_id = Uuid::new_v4();

    // Encrypt BEFORE the write; the column only ever holds 'enc:v1:' ciphertext and a write fails
    // closed (500) when the master key is missing rather than storing the key in the clear.
    let stored_api_key =
        provider_key_crypto::encrypt_optional(&state.pool, req.api_key.as_deref()).await?;

    sqlx::query(
        "INSERT INTO target_software (id, tenant_id, name, webhook_url, api_key) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(target_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.webhook_url)
    .bind(&stored_api_key)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": target_id,
            "message": "Target software created",
            "api_key_masked": req
                .api_key
                .as_deref()
                .map(provider_key_crypto::mask)
                .unwrap_or_default(),
        })),
    ))
}

pub async fn list_routing_logs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<RoutingLog>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let logs = sqlx::query_as::<_, RoutingLog>(
        "SELECT * FROM routing_log WHERE source_tenant = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(logs))
}
