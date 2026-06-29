use sqlx::Row;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use rand::Rng;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::models::api_key::*;
use crate::state::AppState;

fn generate_api_key(prefix_tag: &str) -> (String, String, String) {
    let random_bytes: [u8; 32] = rand::thread_rng().gen();
    let suffix: String = random_bytes.iter().map(|b| format!("{:02x}", b)).collect();
    let raw_key = format!("{}_{}", prefix_tag, suffix);
    let display_prefix = raw_key[..8.min(raw_key.len())].to_string();
    (raw_key, display_prefix, suffix)
}

fn hash_api_key(raw_key: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(raw_key.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Hash error: {e}")))?
        .to_string();
    Ok(hash)
}

pub async fn create_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let (raw_key, prefix, _) = generate_api_key("funnelswift");
    let key_hash = hash_api_key(&raw_key)?;

    let id = Uuid::new_v4();
    let user_id = Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::BadRequest("Invalid user_id in token".into()))?;
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant_id in token".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_api_keys", "API keys").await?;

    let name = req.name.clone().unwrap_or_else(|| "default".into());

    sqlx::query(
        "INSERT INTO api_keys (id, tenant_id, user_id, name, key_hash, prefix, permissions, target_url) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(&name)
    .bind(&key_hash)
    .bind(&prefix)
    .bind(req.permissions.unwrap_or_else(|| serde_json::json!([])))
    .bind(&req.target_url)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "name": name,
            "prefix": prefix,
            "raw_key": raw_key,
            "target_url": req.target_url,
        })),
    ))
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant_id".into()))?;

    let rows = sqlx::query(
        "SELECT id, tenant_id, user_id, name, prefix, permissions, target_url, last_used_at, expires_at, is_active, created_at, updated_at FROM api_keys WHERE tenant_id = $1 ORDER BY created_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let keys: Vec<serde_json::Value> = rows.iter().map(|row| {
        json!({
            "id": row.try_get::<Uuid, _>("id").unwrap_or_default(),
            "name": row.try_get::<String, _>("name").unwrap_or_default(),
            "prefix": row.try_get::<String, _>("prefix").unwrap_or_default(),
            "permissions": row.try_get::<serde_json::Value, _>("permissions").unwrap_or(serde_json::json!([])),
            "target_url": row.try_get::<Option<String>, _>("target_url").unwrap_or(None),
            "last_used_at": row.try_get::<Option<chrono::DateTime<Utc>>, _>("last_used_at").unwrap_or(None),
            "expires_at": row.try_get::<Option<chrono::DateTime<Utc>>, _>("expires_at").unwrap_or(None),
            "is_active": row.try_get::<bool, _>("is_active").unwrap_or(true),
            "created_at": row.try_get::<chrono::DateTime<Utc>, _>("created_at").unwrap_or_default(),
        })
    }).collect();

    Ok(Json(json!({ "data": keys })))
}

pub async fn update_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant_id".into()))?;

    let existing = sqlx::query(
        "SELECT id FROM api_keys WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    if existing.is_none() {
        return Err(AppError::NotFound("API key not found".into()));
    }

    let mut set_parts = Vec::new();
    if let Some(ref name) = req.name {
        let safe = name.replace('\'', "''");
        set_parts.push(format!("name = '{}'", safe));
    }
    if let Some(ref url) = req.target_url {
        let safe = url.replace('\'', "''");
        set_parts.push(format!("target_url = '{}'", safe));
    }
    if let Some(ref perms) = req.permissions {
        let s = perms.to_string().replace('\'', "''");
        set_parts.push(format!("permissions = '{}'::jsonb", s));
    }
    if let Some(active) = req.is_active {
        set_parts.push(format!("is_active = {}", active));
    }
    set_parts.push("updated_at = NOW()".into());

    if !set_parts.is_empty() {
        let sql = format!(
            "UPDATE api_keys SET {} WHERE id = '{}' AND tenant_id = '{}'",
            set_parts.join(", "),
            id,
            tenant_id
        );
        sqlx::query(&sql).execute(&state.pool).await?;
    }

    Ok(Json(json!({ "message": "API key updated", "id": id })))
}

pub async fn delete_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant_id".into()))?;

    let result = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("API key not found".into()));
    }

    Ok(Json(json!({ "message": "API key deleted", "id": id })))
}
