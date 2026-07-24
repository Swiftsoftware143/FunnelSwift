//! Webhook dispatch with retry and exponential backoff.
//!
//! Fires webhooks for configured events and logs deliveries.
//! Failed deliveries are retried with exponential backoff:
//!   Attempt 1 → immediate
//!   Attempt 2 → 30s
//!   Attempt 3 → 120s
//!   Attempt 4+ → 300s (max backoff)
//!
//! Max 5 attempts by default. Configurable per webhook.

use crate::state::AppState;
use reqwest::Client;
use serde_json::{json, Value};
use uuid::Uuid;
use std::time::Duration;
use chrono::Utc;

const MAX_RETRIES: i32 = 5;
const BACKOFF_SECS: &[i64] = &[0, 30, 120, 300, 300]; // attempt 0-indexed

/// Fire a webhook for a given event.
/// Logs delivery and retries on failure with exponential backoff.
pub async fn dispatch_webhook(
    state: &AppState,
    tenant_id: Uuid,
    webhook_id: Uuid,
    webhook_url: &str,
    webhook_secret: Option<&str>,
    event: &str,
    payload: Value,
) {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| Client::new());

    let log_id = Uuid::new_v4();

    // Insert initial delivery log
    let _ = sqlx::query(
        r#"INSERT INTO webhook_delivery_log (id, webhook_id, tenant_id, event, status, request_body, attempt, max_attempts)
           VALUES ($1, $2, $3, $4, 'pending', $5::jsonb, 1, $6)"#
    )
    .bind(log_id)
    .bind(webhook_id)
    .bind(tenant_id)
    .bind(event)
    .bind(&payload)
    .bind(MAX_RETRIES)
    .execute(&state.pool)
    .await;

    // Send the request
    let mut req = client.post(webhook_url).json(&payload);
    if let Some(secret) = webhook_secret {
        req = req.header("X-Webhook-Secret", secret);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();

            if (200..300).contains(&status) {
                // Success
                let _ = sqlx::query(
                    r#"UPDATE webhook_delivery_log SET status = 'success', status_code = $1, response_body = $2, delivered_at = NOW()
                       WHERE id = $3"#
                )
                .bind(status as i32)
                .bind(&body)
                .bind(log_id)
                .execute(&state.pool)
                .await;
                tracing::info!("Webhook {} fired successfully: {} (status {})", webhook_id, event, status);
            } else {
                // Non-2xx — schedule retry
                schedule_retry(&state.pool, log_id, 1, status as i32, &body).await;
                tracing::warn!("Webhook {} returned {}: {} for event {}", webhook_id, status, body, event);
            }
        }
        Err(e) => {
            // Connection error — retry
            schedule_retry(&state.pool, log_id, 1, 0, &e.to_string()).await;
            tracing::warn!("Webhook {} failed: {} for event {}", webhook_id, e, event);
        }
    }
}

/// Schedule a retry for a failed delivery.
async fn schedule_retry(
    pool: &sqlx::PgPool,
    log_id: Uuid,
    attempt: i32,
    status_code: i32,
    response_body: &str,
) {
    let next_attempt = attempt + 1;
    if next_attempt > MAX_RETRIES {
        // Give up
        let _ = sqlx::query(
            r#"UPDATE webhook_delivery_log SET status = 'failed', status_code = $1, response_body = $2, delivered_at = NOW()
               WHERE id = $3"#
        )
        .bind(status_code)
        .bind(response_body)
        .bind(log_id)
        .execute(pool)
        .await;
        tracing::warn!("Webhook delivery {} exhausted {} retries, giving up", log_id, MAX_RETRIES);
        return;
    }

    let delay_secs = BACKOFF_SECS.get(attempt as usize).copied().unwrap_or(300);
    let next_retry = Utc::now() + chrono::Duration::seconds(delay_secs);
    let attempt_idx = attempt + 1;

    let _ = sqlx::query(
        r#"UPDATE webhook_delivery_log SET status = 'failed', status_code = $1, response_body = $2,
           attempt = $3, next_retry_at = $4, delivered_at = NOW()
           WHERE id = $5"#
    )
    .bind(status_code)
    .bind(response_body)
    .bind(attempt_idx)
    .bind(next_retry)
    .bind(log_id)
    .execute(pool)
    .await;

    tracing::info!("Webhook delivery {} scheduled retry #{} in {}s", log_id, attempt_idx, delay_secs);
}

/// Retry all failed webhooks that are due for retry.
/// Intended to be called periodically (e.g., every 60s from a background task).
pub async fn retry_failed_deliveries(state: &AppState) -> i32 {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| Client::new());

    // Find all deliveries due for retry
    let due: Vec<(Uuid, Uuid, String, String, Value, i32)> = sqlx::query_as::<_, (Uuid, Uuid, String, String, Value, i32)>(
        r#"SELECT dl.id, w.id, w.url, dl.event, COALESCE(dl.request_body, '{}'::jsonb), dl.attempt
           FROM webhook_delivery_log dl
           JOIN webhooks w ON w.id = dl.webhook_id
           WHERE dl.status = 'failed'
             AND dl.next_retry_at <= NOW()
             AND dl.attempt < dl.max_attempts
           LIMIT 50"#,
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let count = due.len() as i32;

    for (log_id, webhook_id, url, event, payload, attempt) in due {
        // Update status to retrying
        let _ = sqlx::query(
            "UPDATE webhook_delivery_log SET status = 'retrying' WHERE id = $1"
        )
        .bind(log_id)
        .execute(&state.pool)
        .await;

        let req = client.post(&url).json(&payload);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                if (200..300).contains(&status) {
                    let _ = sqlx::query(
                        r#"UPDATE webhook_delivery_log SET status = 'success', status_code = $1, response_body = $2, delivered_at = NOW()
                           WHERE id = $3"#
                    )
                    .bind(status as i32)
                    .bind(&body)
                    .bind(log_id)
                    .execute(&state.pool)
                    .await;
                    tracing::info!("Webhook retry {} succeeded (attempt {})", webhook_id, attempt + 1);
                } else {
                    schedule_retry(&state.pool, log_id, attempt, status as i32, &body).await;
                }
            }
            Err(e) => {
                schedule_retry(&state.pool, log_id, attempt, 0, &e.to_string()).await;
            }
        }
    }

    count
}

/// Fire all active webhooks for a given event on a tenant.
pub async fn fire_event_webhooks(
    state: &AppState,
    tenant_id: Uuid,
    event: &str,
    payload: Value,
) {
    // Find all active webhooks subscribed to this event
    let webhooks: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        r#"SELECT id, url, secret FROM webhooks
           WHERE tenant_id = $1 AND is_active = true AND events @> $2::jsonb"#
    )
    .bind(tenant_id)
    .bind(json!([event]))
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    for (webhook_id, url, secret) in webhooks {
        dispatch_webhook(
            state,
            tenant_id,
            webhook_id,
            &url,
            secret.as_deref(),
            event,
            payload.clone(),
        ).await;
    }
}
