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
use crate::models::webhook::*;
use crate::state::AppState;

pub async fn list_webhooks(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Webhook>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let webhooks = sqlx::query_as::<_, Webhook>(
        "SELECT * FROM webhooks WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(webhooks))
}

pub async fn create_webhook(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateWebhookRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_webhooks", "Webhooks").await?;
    let webhook_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO webhooks (id, tenant_id, name, url, events, secret) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(webhook_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.url)
    .bind(serde_json::to_value(&req.events).map_err(|e| AppError::Internal(format!("Events serialize error: {e}")))?)
    .bind(&req.secret)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": webhook_id, "message": "Webhook created"}))))
}

pub async fn delete_webhook(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("DELETE FROM webhooks WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Webhook deleted"})))
}

pub async fn test_webhook(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let webhook = sqlx::query_as::<_, Webhook>(
        "SELECT * FROM webhooks WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Webhook not found".into()))?;

    let client = reqwest::Client::new();
    let payload = json!({"event": "test", "message": "This is a test webhook from FunnelSwift"});

    let result = client
        .post(&webhook.url)
        .json(&payload)
        .send()
        .await;

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Ok(Json(json!({
                "status": status,
                "response": body,
                "message": "Webhook test completed"
            })))
        }
        Err(e) => {
            Ok(Json(json!({
                "status": "error",
                "error": e.to_string(),
                "message": "Webhook test failed"
            })))
        }
    }
}
