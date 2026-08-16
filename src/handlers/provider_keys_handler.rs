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

pub async fn list_provider_keys(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows: Vec<(String, String, Option<String>, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT provider, COALESCE(api_key,''), base_url, created_at FROM provider_keys WHERE tenant_id = $1 ORDER BY provider"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn upsert_provider_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let provider = payload["provider"]
        .as_str()
        .ok_or_else(|| AppError::Validation("provider required".into()))?;
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("INSERT INTO provider_keys (id, tenant_id, provider, api_key, base_url) VALUES ($1, $2, $3, $4, $5)")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(provider)
        .bind(payload["api_key"].as_str().unwrap_or("")).bind(payload["base_url"].as_str().unwrap_or(""))
        .execute(&state.pool).await?;
    Ok((StatusCode::OK, Json(json!({"message": "Key saved"}))))
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
    Ok(Json(json!({"message": "Key deleted"})))
}
pub async fn list_available_providers(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        "stripe",
        "paypal",
        "openai",
        "gemini",
        "anthropic",
        "sendgrid",
        "twilio"
    ])))
}
