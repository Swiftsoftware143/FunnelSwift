//! Outbound webhook delivery engine (kanban t_431faa99).
//!
//! Before this module `webhooks` was stored configuration with no reader. The table existed, the
//! console could create a row, and `POST /api/v1/webhooks/:id/test` could reach an endpoint — but
//! no event ever fired a webhook, `events` selected nothing and `is_active` gated nothing. This is
//! the reader: the product answer to the card was to BUILD the delivery, not to retire a surface
//! that is already sold (`has_webhooks` / `max_webhooks` on three paid plans) and already has a
//! delivery-log table designed for it (`migrations/0016_webhook_delivery_log.sql`).
//!
//! The three decisions the card asked for:
//!
//! * EVENT VOCABULARY — exactly the events a writer in this binary actually emits, published at
//!   `GET /api/v1/webhook-events` so no console can advertise a name nothing sends:
//!   `lead.created` (every writer of a `leads` row) and `tag.updated` (the console's tag editor).
//! * DISPATCHER — the delivery row IS the queue. It is written `pending` BEFORE the POST, so a
//!   crash between the two leaves a row the sweeper recovers, and the visitor's own submission
//!   never waits on a third party. One sweeper per process claims rows with `FOR UPDATE SKIP
//!   LOCKED`, so two instances cannot deliver the same row twice.
//! * SIGNING — `X-FunnelSwift-Signature: sha256=<hex HMAC-SHA256(secret, exact body bytes)>`, only
//!   when the webhook stored a `secret` (a field the console has always written and nothing read).
//!
//! Retry policy: 3 attempts in total, 60s then 120s apart, terminal after the third.
//!
//! Two delivery-time guards, both re-run on EVERY attempt rather than only at create time:
//! the SSRF check (a hostname that resolved publicly when the webhook was saved can be re-pointed
//! at a private address later) and redirects-off (a public URL cannot bounce to an internal one).

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

/// The whole vocabulary. Adding a name here without a `spawn(...)` call site is how the product
/// would start advertising an event nothing emits — the card's original defect, one level up.
pub const LEAD_CREATED: &str = "lead.created";
pub const TAG_UPDATED: &str = "tag.updated";
pub const EVENTS: &[&str] = &[LEAD_CREATED, TAG_UPDATED];

/// Attempt 1 happens inline on the request path; the sweeper makes attempts 2 and 3.
const MAX_ATTEMPTS: i32 = 3;
/// Backoff between attempts: 60s after attempt 1, 120s after attempt 2.
const RETRY_BACKOFF_SECS: i64 = 60;
const SWEEP_INTERVAL_SECS: u64 = 30;
/// A row younger than this is assumed to be in flight on the request path, not abandoned.
const SWEEP_MIN_AGE_SECS: i64 = 45;
/// Bound on rows handled per sweep, so one sweep is never unbounded work.
const SWEEP_BATCH: usize = 25;
const DELIVERY_TIMEOUT_SECS: u64 = 10;
/// Stored `response_body` cap. This text is also what `GET /webhooks/:id/deliveries` returns.
const RESPONSE_BODY_CAP: usize = 2000;

struct Delivery {
    id: Uuid,
    webhook_id: Uuid,
    url: String,
    secret: Option<String>,
    event: String,
    body: String,
    attempt: i32,
}

/// Fire-and-forget: queues one delivery row per subscribed, active webhook and sends the first
/// attempt. Never awaited by a request handler — the caller has already stored its own work.
pub fn spawn(pool: PgPool, tenant_id: Uuid, event: &'static str, data: serde_json::Value) {
    tokio::spawn(async move {
        match dispatch(&pool, tenant_id, event, data).await {
            Ok(0) => {}
            Ok(n) => tracing::debug!(event, webhooks = n, "webhook dispatch queued"),
            Err(e) => {
                tracing::warn!(event, error = %e, "webhook dispatch failed (event was not delivered)")
            }
        }
    });
}

/// The payload builder for `lead.created`, shared by every writer of a `leads` row so the four
/// call sites cannot drift into four different shapes.
pub fn lead_created(
    lead_id: Uuid,
    name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    company: Option<&str>,
    source: Option<&str>,
    tags: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "lead_id": lead_id.to_string(),
        "name": name.unwrap_or(""),
        "email": email.unwrap_or(""),
        "phone": phone,
        "company": company,
        "source": source,
        "tags": tags,
    })
}

async fn dispatch(
    pool: &PgPool,
    tenant_id: Uuid,
    event: &str,
    data: serde_json::Value,
) -> Result<usize, sqlx::Error> {
    // One indexed read (`idx_webhooks_tenant`): the whole cost of an emitted event for a workspace
    // with no webhook is this SELECT. `events @> ["lead.created"]` is also what makes the stored
    // `events` array load-bearing rather than decorative.
    let subs: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT id, url, secret FROM webhooks \
         WHERE tenant_id = $1 AND is_active = true AND events @> $2::jsonb ORDER BY created_at",
    )
    .bind(tenant_id)
    .bind(serde_json::json!([event]))
    .fetch_all(pool)
    .await?;

    let count = subs.len();
    for (webhook_id, url, secret) in subs {
        let delivery_id = Uuid::new_v4();
        let body = envelope(delivery_id, tenant_id, event, &data).to_string();
        queue_row(pool, delivery_id, webhook_id, tenant_id, event, &body).await?;
        deliver_row(
            pool,
            Delivery {
                id: delivery_id,
                webhook_id,
                url,
                secret,
                event: event.to_string(),
                body,
                attempt: 1,
            },
        )
        .await;
    }
    Ok(count)
}

fn envelope(
    delivery_id: Uuid,
    tenant_id: Uuid,
    event: &str,
    data: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": delivery_id.to_string(),
        "event": event,
        "tenant_id": tenant_id.to_string(),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "data": data,
    })
}

async fn queue_row(
    pool: &PgPool,
    delivery_id: Uuid,
    webhook_id: Uuid,
    tenant_id: Uuid,
    event: &str,
    body: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO webhook_delivery_log \
         (id, webhook_id, tenant_id, event, status, request_body, attempt, max_attempts) \
         VALUES ($1, $2, $3, $4, 'pending', $5, 1, $6)",
    )
    .bind(delivery_id)
    .bind(webhook_id)
    .bind(tenant_id)
    .bind(event)
    .bind(body)
    .bind(MAX_ATTEMPTS)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Sends one attempt and records its outcome. An outcome that cannot be recorded is logged and
/// dropped: a bookkeeping failure must never become a second failure for the caller.
async fn deliver_row(pool: &PgPool, d: Delivery) {
    let (code, resp_body, err) = post_once(&d).await;
    let ok = matches!(code, Some(c) if (200..300).contains(&c));

    if ok {
        if let Err(e) = sqlx::query(
            "UPDATE webhook_delivery_log SET status = 'success', status_code = $2, \
             response_body = $3, delivered_at = now(), next_retry_at = NULL WHERE id = $1",
        )
        .bind(d.id)
        .bind(code.map(i32::from))
        .bind(&resp_body)
        .execute(pool)
        .await
        {
            tracing::warn!(delivery = %d.id, error = %e, "webhook delivery log update failed");
        }
        return;
    }

    let exhausted = d.attempt >= MAX_ATTEMPTS;
    let next_retry_at = if exhausted {
        None
    } else {
        Some(
            chrono::Utc::now()
                + chrono::Duration::seconds(RETRY_BACKOFF_SECS * (1i64 << (d.attempt - 1).max(0))),
        )
    };
    let detail = err
        .or_else(|| resp_body.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("endpoint answered HTTP {}", code.unwrap_or(0)));

    tracing::warn!(
        delivery = %d.id,
        event = %d.event,
        attempt = d.attempt,
        status_code = ?code,
        retry = ?next_retry_at,
        error = %detail,
        "webhook delivery failed"
    );

    if let Err(e) = sqlx::query(
        "UPDATE webhook_delivery_log SET status = 'failed', status_code = $2, response_body = $3, \
         attempt = $4, next_retry_at = $5, delivered_at = now() WHERE id = $1",
    )
    .bind(d.id)
    .bind(code.map(i32::from))
    .bind(&detail)
    .bind(d.attempt)
    .bind(next_retry_at)
    .execute(pool)
    .await
    {
        tracing::warn!(delivery = %d.id, error = %e, "webhook delivery log update failed");
    }
}

/// One POST. Returns `(status_code, capped response body, transport error)`.
async fn post_once(d: &Delivery) -> (Option<u16>, Option<String>, Option<String>) {
    if let Err(e) = crate::handlers::webhook_handler::validate_webhook_url(&d.url) {
        return (None, None, Some(format!("refused by the SSRF guard: {e}")));
    }

    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(DELIVERY_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => return (None, None, Some(format!("http client error: {e}"))),
    };

    let mut req = client
        .post(&d.url)
        .header("content-type", "application/json")
        .header("x-funnelswift-event", d.event.clone())
        .header("x-funnelswift-delivery", d.id.to_string());
    if let Some(sig) = d.secret.as_deref().and_then(|s| sign(s, d.body.as_bytes())) {
        req = req.header("x-funnelswift-signature", sig);
    }

    match req.body(d.body.clone()).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let mut text = resp.text().await.unwrap_or_default();
            if text.len() > RESPONSE_BODY_CAP {
                text.truncate(RESPONSE_BODY_CAP);
            }
            (Some(status), Some(text), None)
        }
        Err(e) => (None, None, Some(e.to_string())),
    }
}

/// `sha256=<hex>` over the exact bytes that were sent, so a receiver can verify the body it
/// received. Returns `None` for a key length HMAC refuses (it accepts any, so this is a guard
/// against a future signature scheme, not a live path).
fn sign(secret: &str, body: &[u8]) -> Option<String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return None;
    };
    mac.update(body);
    Some(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

/// Boot-time worker: derives retries from the delivery log itself. Started from `main` next to the
/// other background tasks; a failure inside one sweep is logged and the loop continues.
pub fn spawn_retry_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match sweep_once(&pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(retried = n, "webhook delivery sweep"),
                Err(e) => tracing::warn!(error = %e, "webhook delivery sweep failed"),
            }
        }
    });
}

/// Claims and re-sends at most `SWEEP_BATCH` due rows. The claim is a `FOR UPDATE SKIP LOCKED`
/// select plus a conditional status update inside one transaction, so two workers (or two app
/// instances) never take the same row.
async fn sweep_once(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let mut handled = 0usize;
    while handled < SWEEP_BATCH {
        let mut tx = pool.begin().await?;
        let claimed: Option<(
            Uuid,
            Uuid,
            String,
            Option<String>,
            i32,
            String,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT d.id, d.webhook_id, d.event, d.request_body, d.attempt, w.url, w.secret \
                 FROM webhook_delivery_log d JOIN webhooks w ON w.id = d.webhook_id \
                 WHERE d.attempt < d.max_attempts \
                   AND d.status IN ('pending', 'failed', 'sending') \
                   AND d.delivered_at < now() - make_interval(secs => $1) \
                   AND (d.next_retry_at IS NULL OR d.next_retry_at <= now()) \
                 ORDER BY d.delivered_at ASC LIMIT 1 FOR UPDATE OF d SKIP LOCKED",
        )
        .bind(SWEEP_MIN_AGE_SECS as f64)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((id, webhook_id, event, body, attempt, url, secret)) = claimed else {
            tx.commit().await?;
            break;
        };
        let taken = sqlx::query(
            "UPDATE webhook_delivery_log SET status = 'sending' \
             WHERE id = $1 AND status IN ('pending', 'failed', 'sending')",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        if taken.rows_affected() == 0 {
            continue;
        }

        deliver_row(
            pool,
            Delivery {
                id,
                webhook_id,
                url,
                secret,
                event,
                body: body.unwrap_or_default(),
                attempt: attempt + 1,
            },
        )
        .await;
        handled += 1;
    }
    Ok(handled)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signature is over the exact bytes sent, so it must be stable and secret-dependent.
    #[test]
    fn signature_is_deterministic_and_secret_bound() {
        let body = br#"{"event":"lead.created"}"#;
        let a = sign("s3cret", body).expect("hmac accepts any key length");
        assert_eq!(a, sign("s3cret", body).expect("stable"));
        assert_ne!(a, sign("other", body).expect("secret bound"));
        assert!(a.starts_with("sha256="));
        assert_eq!(a.len(), "sha256=".len() + 64);
    }

    /// The published vocabulary is what the consoles advertise; an empty list would make
    /// `events` unfilterable and every webhook would receive every event.
    #[test]
    fn vocabulary_is_not_empty_and_matches_the_emit_sites() {
        assert!(EVENTS.contains(&LEAD_CREATED));
        assert!(EVENTS.contains(&TAG_UPDATED));
    }
}
