//! BYOK provider credentials + the Integration Center catalogue.
//!
//! Fleet standard: /opt/swift/docs/integration-center-standard-2026-09-20.md
//!   available_providers = the catalogue the Integration Center renders (never a hardcoded
//!   array in the SPA); provider_keys = USER/tenant-level BYOK credentials.
//!
//! Keys are user-level and read back MASKED (prefix + last 4) — the raw key never leaves the
//! write path.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// Mask a stored key for read-back: first 8 + last 4.
fn mask(api_key: &str) -> String {
    let k = api_key.trim();
    if k.is_empty() {
        return String::new();
    }
    if k.len() <= 12 {
        return "****".to_string();
    }
    format!("{}…{}", &k[..8], &k[k.len() - 4..])
}

pub async fn list_provider_keys(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows: Vec<(
        String,
        String,
        Option<String>,
        bool,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT provider, COALESCE(api_key,''), base_url, is_active, created_at \
             FROM provider_keys WHERE tenant_id = $1 ORDER BY provider",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(provider, api_key, base_url, is_active, created_at)| {
            json!({
                "provider": provider,
                "api_key_masked": mask(&api_key),
                "base_url": base_url,
                "is_active": is_active,
                "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

pub async fn upsert_provider_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let provider = payload["provider"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Validation("provider required".into()))?;
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();

    // A key is required unless the tenant is only recording a base URL for a provider that
    // already has one (the Integration Center's "update endpoint" flow).
    let api_key = payload["api_key"].as_str().unwrap_or("").trim().to_string();
    let base_url = payload["base_url"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let mut metadata = payload
        .get("metadata")
        .cloned()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| json!({}));

    let exists: Option<String> = sqlx::query_scalar(
        "SELECT COALESCE(api_key,'') FROM provider_keys WHERE tenant_id = $1 AND provider = $2",
    )
    .bind(tenant_id)
    .bind(&provider)
    .fetch_optional(&state.pool)
    .await?;

    match (&exists, api_key.is_empty()) {
        (None, true) => {
            return Err(AppError::Validation("api_key required".into()));
        }
        (Some(prev), true) => {
            // Keep the stored secret, only touch base_url / metadata.
            metadata["key_unchanged"] = json!(true);
            sqlx::query(
                "UPDATE provider_keys SET base_url = $3, metadata = metadata || $4::jsonb, \
                 is_active = true, updated_at = NOW() \
                 WHERE tenant_id = $1 AND provider = $2",
            )
            .bind(tenant_id)
            .bind(&provider)
            .bind(&base_url)
            .bind(&metadata)
            .execute(&state.pool)
            .await?;
            let _ = prev;
            return Ok((
                StatusCode::OK,
                Json(json!({
                    "message": "Key saved",
                    "provider": provider,
                    "api_key_masked": mask(
                        &sqlx::query_scalar::<_, String>(
                            "SELECT COALESCE(api_key,'') FROM provider_keys \
                             WHERE tenant_id = $1 AND provider = $2",
                        )
                        .bind(tenant_id)
                        .bind(&provider)
                        .fetch_one(&state.pool)
                        .await?
                    ),
                })),
            ));
        }
        _ => {}
    }

    sqlx::query(
        "INSERT INTO provider_keys (id, tenant_id, provider, api_key, base_url, metadata, is_active) \
         VALUES ($1, $2, $3, $4, $5, $6, true) \
         ON CONFLICT (tenant_id, provider) DO UPDATE SET \
           api_key = EXCLUDED.api_key, base_url = EXCLUDED.base_url, \
           metadata = provider_keys.metadata || EXCLUDED.metadata, \
           is_active = true, updated_at = NOW()",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(&provider)
    .bind(&api_key)
    .bind(&base_url)
    .bind(&metadata)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "message": "Key saved",
            "provider": provider,
            "api_key_masked": mask(&api_key),
        })),
    ))
}

pub async fn delete_provider_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("DELETE FROM provider_keys WHERE provider = $1 AND tenant_id = $2")
        .bind(&provider)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(
        json!({ "message": "Key deleted", "provider": provider }),
    ))
}

/// The Integration Center catalogue — read from `available_providers`, never hardcoded.
/// CoreSwift is pinned first (standard §R2 / §UI contract).
pub async fn list_available_providers(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let rows: Vec<(String, String, Option<String>, bool, Value, Option<String>)> = sqlx::query_as(
        "SELECT key, name, description, requires_base_url, requires_metadata, icon \
         FROM available_providers \
         ORDER BY (key = 'coreswift') DESC, name",
    )
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(
            |(key, name, description, requires_base_url, requires_metadata, icon)| {
                json!({
                    "key": key,
                    "name": name,
                    "description": description.unwrap_or_default(),
                    "requires_base_url": requires_base_url,
                    "requires_metadata": requires_metadata,
                    "icon": icon,
                    "native": key == "coreswift" || key == "incentiveswift",
                })
            },
        )
        .collect();
    Ok(Json(json!(out)))
}
