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

pub async fn list_payment_providers(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows: Vec<(Uuid, Option<Uuid>, String, String, bool, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, tenant_id, provider_type, COALESCE(api_key,''), is_active, created_at FROM payment_providers WHERE tenant_id = $1 ORDER BY created_at"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn upsert_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let provider = payload["provider_type"]
        .as_str()
        .ok_or_else(|| AppError::Validation("provider_type required".into()))?;
    let api_key = payload["api_key"].as_str().unwrap_or("");
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("INSERT INTO payment_providers (id, tenant_id, provider_type, api_key, is_active) VALUES ($1, $2, $3, $4, true)")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(provider).bind(api_key).execute(&state.pool).await?;
    Ok((StatusCode::OK, Json(json!({"message": "Provider saved"}))))
}
pub async fn delete_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider_type): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("DELETE FROM payment_providers WHERE provider_type = $1 AND tenant_id = $2")
        .bind(&provider_type)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Provider deleted"})))
}
pub async fn create_checkout_session(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((
        StatusCode::OK,
        Json(
            json!({"session_id": "cs_test_placeholder", "url": "https://checkout.stripe.com/c/pay/placeholder"}),
        ),
    ))
}
pub async fn list_checkout_sessions(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!([])))
}
pub async fn stripe_webhook(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::OK, Json(json!({"received": true}))))
}
pub async fn paypal_webhook(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::OK, Json(json!({"received": true}))))
}
