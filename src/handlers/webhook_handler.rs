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

/// SSRF guard: webhook URLs must be http(s), must not target loopback/private/link-local
/// addresses, the cloud metadata endpoint, or internal hostnames, and must be resolvable.
fn validate_webhook_url(url: &str) -> AppResult<()> {
    use std::net::ToSocketAddrs;

    let parsed =
        reqwest::Url::parse(url).map_err(|_| AppError::BadRequest("Invalid webhook URL".into()))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(AppError::BadRequest(
                "Webhook URL must use http or https".into(),
            ))
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::BadRequest("Webhook URL is missing a host".into()))?;
    let host_lower = host.to_ascii_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".internal")
        || host_lower == "metadata.google.internal"
    {
        return Err(AppError::BadRequest(
            "Webhook URL points to a forbidden host".into(),
        ));
    }

    let port = parsed.port_or_known_default().unwrap_or(443);
    let mut resolved_any = false;
    if let Ok(addrs) = (host, port).to_socket_addrs() {
        for addr in addrs {
            resolved_any = true;
            match addr.ip() {
                std::net::IpAddr::V4(v4)
                    if v4.is_private()
                        || v4.is_loopback()
                        || v4.is_link_local()
                        || v4.is_unspecified()
                        || v4.is_broadcast() =>
                {
                    return Err(AppError::BadRequest(
                        "Webhook URL resolves to a private/loopback address".into(),
                    ))
                }
                std::net::IpAddr::V6(v6) if v6.is_loopback() || v6.is_unspecified() => {
                    return Err(AppError::BadRequest(
                        "Webhook URL resolves to a private/loopback address".into(),
                    ))
                }
                _ => {}
            }
        }
    }
    if !resolved_any {
        return Err(AppError::BadRequest(
            "Webhook URL could not be resolved".into(),
        ));
    }
    Ok(())
}

pub async fn list_webhooks(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Webhook>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

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
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    validate_webhook_url(&req.url)?;
    features::enforce_feature_flag(&state, tenant_id, "has_webhooks", "Webhooks").await?;
    features::enforce_feature_limit(&state, tenant_id, "max_webhooks", "Webhooks").await?;
    let webhook_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO webhooks (id, tenant_id, name, url, events, secret, is_active) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(webhook_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.url)
    .bind(serde_json::to_value(&req.events).map_err(|e| AppError::Internal(format!("Events serialize error: {e}")))?)
    .bind(&req.secret)
    // kanban t_ae907da8: the Add form's Active toggle used to be dropped on the floor (the key was
    // not in the request struct, so serde ignored it and every webhook was created active).
    .bind(req.is_active.unwrap_or(true))
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": webhook_id, "message": "Webhook created"})),
    ))
}

/// `PUT /api/v1/webhooks/:id` — the Webhook editor's Save (kanban t_ae907da8).
///
/// The route was never registered (only DELETE lived on this path), so the console's editor could
/// not save anything; the arm taken was WIRE, not REMOVE: the screen's list + Add + Test paths are
/// all live and the resource is plan-gated (`has_webhooks` / `max_webhooks`), so deleting the
/// editor would have left the only editable field of a sold feature unreachable.
///
/// Two deliberate properties:
/// * an absent key KEEPS the stored value, so a no-touch save is a no-op that still answers 200
///   (measured: nothing in the console's form is required to change to press Save);
/// * the URL is validated only when the request actually CHANGES it. Re-validating the stored URL on
///   every save would refuse to rename a webhook whose hostname has stopped resolving — the same
///   "read path wedged by a write guard" shape as the max_cards defect on t_8ccc8e6a.
pub async fn update_webhook(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateWebhookRequest>,
) -> AppResult<Json<Webhook>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    if let Some(url) = req.url.as_deref() {
        validate_webhook_url(url)?;
    }

    let existing =
        sqlx::query_as::<_, Webhook>("SELECT * FROM webhooks WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Webhook not found".into()))?;

    let name = req.name.unwrap_or(existing.name);
    let url = req.url.unwrap_or(existing.url);
    let events = match req.events {
        Some(events) => serde_json::to_value(&events)
            .map_err(|e| AppError::Internal(format!("Events serialize error: {e}")))?,
        None => existing.events,
    };
    let is_active = req.is_active.unwrap_or(existing.is_active);

    let updated = sqlx::query_as::<_, Webhook>(
        "UPDATE webhooks SET name = $1, url = $2, events = $3, is_active = $4, updated_at = NOW()
         WHERE id = $5 AND tenant_id = $6
         RETURNING *",
    )
    .bind(&name)
    .bind(&url)
    .bind(events)
    .bind(is_active)
    .bind(id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(updated))
}

pub async fn delete_webhook(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

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
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let webhook =
        sqlx::query_as::<_, Webhook>("SELECT * FROM webhooks WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Webhook not found".into()))?;

    validate_webhook_url(&webhook.url)?;

    // Disable redirects so a public URL can't bounce to an internal address.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {e}")))?;
    let payload = json!({"event": "test", "message": "This is a test webhook from FunnelSwift"});

    match client.post(&webhook.url).json(&payload).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // Never echo the response body back to the caller (SSRF data-exfil guard).
            Ok(Json(json!({
                "status": status,
                "message": "Webhook test completed"
            })))
        }
        Err(e) => Ok(Json(json!({
            "status": "error",
            "error": e.to_string(),
            "message": "Webhook test failed"
        }))),
    }
}
