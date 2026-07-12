use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub is_active: bool,
    pub scope: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct MaskedProviderKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub is_active: bool,
    pub scope: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn mask_key(key: &str) -> String {
    if key.len() <= 6 {
        return String::from("**");
    }
    let first3 = &key[..3];
    let last3 = &key[key.len()-3..];
    format!("{}...{}", first3, last3)
}

#[derive(Debug, Deserialize)]
pub struct UpsertProviderKeyRequest {
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AvailableProvider {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub requires_base_url: bool,
    pub requires_metadata: Option<Value>,
    pub icon: Option<String>,
}

pub async fn list_provider_keys(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<MaskedProviderKey>>> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let keys = sqlx::query_as::<_, ProviderKey>(
        "SELECT * FROM provider_keys WHERE tenant_id = $1 ORDER BY provider"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let masked: Vec<MaskedProviderKey> = keys.into_iter().map(|k| {
        let api_key = mask_key(&k.api_key);
        MaskedProviderKey {
            id: k.id,
            tenant_id: k.tenant_id,
            provider: k.provider,
            api_key,
            base_url: k.base_url,
            metadata: k.metadata,
            is_active: k.is_active,
            scope: k.scope,
            created_at: k.created_at,
            updated_at: k.updated_at,
        }
    }).collect();

    Ok(Json(masked))
}

pub async fn upsert_provider_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpsertProviderKeyRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let scope = req.scope.unwrap_or_else(|| "tenant".to_string());

    sqlx::query(
        "INSERT INTO provider_keys (tenant_id, provider, api_key, base_url, metadata, scope) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (tenant_id, provider) DO UPDATE SET api_key = EXCLUDED.api_key, base_url = COALESCE(EXCLUDED.base_url, provider_keys.base_url), metadata = COALESCE(EXCLUDED.metadata, provider_keys.metadata), scope = EXCLUDED.scope, updated_at = NOW()"
    )
    .bind(tenant_id)
    .bind(&req.provider)
    .bind(&req.api_key)
    .bind(&req.base_url)
    .bind(&req.metadata)
    .bind(&scope)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::OK, Json(json!({"message": "Provider key saved", "provider": req.provider}))))
}

pub async fn delete_provider_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("DELETE FROM provider_keys WHERE tenant_id = $1 AND provider = $2")
        .bind(tenant_id)
        .bind(&provider)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Provider key deleted", "provider": provider})))
}

pub async fn list_available_providers(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<AvailableProvider>>> {
    let rows = sqlx::query(
        "SELECT key, name, description, requires_base_url, requires_metadata, icon FROM available_providers ORDER BY name"
    )
    .fetch_all(&state.pool)
    .await?;

    let providers: Vec<AvailableProvider> = rows.iter().map(|row| {
        AvailableProvider {
            key: row.get("key"),
            name: row.get("name"),
            description: row.get("description"),
            requires_base_url: row.get("requires_base_url"),
            requires_metadata: row.get("requires_metadata"),
            icon: row.get("icon"),
        }
    }).collect();

    Ok(Json(providers))
}
