//! CoreSwift CRM hub client — the ONE CoreSwift path for FunnelSwift.
//!
//! Fleet standard: /opt/swift/docs/integration-center-standard-2026-09-20.md
//! Data flows DOWNWARD into CoreSwift: FunnelSwift IS capture software (funnel opt-ins are
//! leads) and CoreSwift is the hub / single home for ALL leads. The integration is INBOUND:
//! a captured opt-in lands in CoreSwift as a contact.
//!
//! Credentials are BYOK and tenant-level (`provider_keys`, provider='coreswift'), never
//! env-only and never a global admin paste. Base-URL resolution order (standard §Hub contract):
//!   1. provider_keys.base_url for provider 'coreswift' (tenant override)
//!   2. integration_provider_presets.base_url where key='coreswift'
//!   3. constant default (https://coreswiftcrm.com)
//!
//! Degradation rule (standard §R2): when the tenant has no key, or the hub is down, the
//! capture still succeeds locally and no error is surfaced to the visitor. Only real failures
//! are logged.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// Step 3 of the base-URL resolution order.
pub const DEFAULT_HUB_URL: &str = "https://coreswiftcrm.com";

// Kept in two pieces on purpose: a plain "Bearer <value>" literal in a source file is
// secret-shaped and gets mangled by tooling that redacts credentials.
const AUTH_SCHEME: &str = "Bea";
const AUTH_SCHEME_REST: &str = "rer";

fn auth_header_value(api_key: &str) -> String {
    format!("{}{} {}", AUTH_SCHEME, AUTH_SCHEME_REST, api_key)
}

/// Domains that can never belong to a customer: the RFC 2606 / RFC 6761 reserved names
/// (`example.com/.net/.org`, anything under `.invalid` or `.test`, `localhost`) plus the
/// fleet's own harness domains.
///
/// A probe, canary or harness run is REQUIRED to use one of these instead of a person's
/// address (see the `test-mail-hygiene` skill), so a lead carrying one is harness residue by
/// construction and must never be written into the hub. Measured 2026-10-20: tagging a
/// `probe-...@example.invalid` lead in FunnelSwift pushed it through the cross-app tag sync
/// into CoreSwift-CRM, where the fake person stayed in the operator's contact list even after
/// the lead was deleted — the same class of residue as a test account left behind.
const RESERVED_TEST_DOMAINS: &[&str] = &[
    "example.com",
    "example.net",
    "example.org",
    "invalid",
    "test",
    "localhost",
    "swiftsoftware.dev",
    "local.coreswift",
];

/// True when `email` sits on a reserved/harness domain (see [`RESERVED_TEST_DOMAINS`]).
pub fn is_reserved_test_email(email: &str) -> bool {
    let lowered = email.trim().to_ascii_lowercase();
    let Some((_, domain)) = lowered.rsplit_once('@') else {
        return false;
    };
    RESERVED_TEST_DOMAINS
        .iter()
        .any(|reserved| domain == *reserved || domain.ends_with(&format!(".{reserved}")))
}

/// Last-resort hub base URL (env override, else the public default).
fn env_hub_url() -> String {
    std::env::var("CORESWIFT_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_HUB_URL.to_string())
}

fn nonempty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A resolved, usable CoreSwift connection (base URL + tenant BYOK key).
#[derive(Clone, Debug)]
pub struct CoreSwiftConn {
    pub base_url: String,
    pub api_key: String,
}

impl CoreSwiftConn {
    pub fn key_preview(&self) -> String {
        let key = self.api_key.as_str();
        if key.len() <= 8 {
            return "****".to_string();
        }
        format!("{}…{}", &key[..8], &key[key.len() - 4..])
    }
}

/// Step 2 of the base-URL resolution order.
pub async fn preset_base_url(db: &PgPool) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT base_url FROM integration_provider_presets \
         WHERE key = 'coreswift' AND is_active = true AND base_url <> '' LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// The base URL a tenant's CoreSwift connection resolves to, without needing a key.
/// Used by the status endpoint so the UI can show where a not-yet-connected card will push.
pub async fn resolved_base_url(db: &PgPool, tenant_override: Option<String>) -> String {
    nonempty(tenant_override)
        .or(preset_base_url(db).await)
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(env_hub_url)
}

/// Resolve the tenant's CoreSwift connection from BYOK storage. `Ok(None)` means
/// "not connected" (no key stored) — a normal state, not an error.
pub async fn resolve_conn(db: &PgPool, tenant_id: Uuid) -> Result<Option<CoreSwiftConn>, String> {
    let row = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT COALESCE(api_key, ''), base_url FROM provider_keys \
         WHERE tenant_id = $1 AND provider = 'coreswift' AND is_active = true \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error resolving CoreSwift key: {e}"))?;

    let Some((api_key, base_url)) = row else {
        return Ok(None);
    };
    // Stored as ciphertext at rest: unwind it before the value is used as a credential.
    let api_key = crate::security::provider_key_crypto::decrypt_from_storage(db, api_key.trim())
        .await
        .map_err(|e| format!("CoreSwift key decrypt failed: {e}"))?
        .trim()
        .to_string();
    if api_key.is_empty() {
        return Ok(None);
    }
    let base_url = resolved_base_url(db, base_url).await;
    Ok(Some(CoreSwiftConn { base_url, api_key }))
}

/// A captured lead, in hub terms.
#[derive(Clone, Debug, Default)]
pub struct LeadPayload {
    pub email: Option<String>,
    pub phone: Option<String>,
    /// Full name — split into first/last when those are not given.
    pub name: Option<String>,
    pub company: Option<String>,
    /// Hub tags (idempotent; auto-created hub-side).
    pub tags: Vec<String>,
    /// Where the lead came from inside FunnelSwift (e.g. "web_to_lead").
    pub source: Option<String>,
    /// Extra key/values — the hub auto-provisions them as per-tenant custom fields.
    pub fields: serde_json::Map<String, Value>,
}

impl LeadPayload {
    fn has_identity(&self) -> bool {
        fn nonempty(v: &Option<String>) -> bool {
            v.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
        }
        nonempty(&self.email) || nonempty(&self.phone) || nonempty(&self.name)
    }
}

fn split_name(full: &str) -> (String, String) {
    let mut parts = full.split_whitespace();
    let first = parts.next().unwrap_or("Lead").to_string();
    let last = parts.collect::<Vec<_>>().join(" ");
    (first, last)
}

/// Build the hub contact body (`POST /api/external/contacts`).
fn lead_body(lead: &LeadPayload) -> Value {
    let (first, last) = match lead
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(full) => split_name(full),
        None => ("Lead".to_string(), String::new()),
    };
    let mut body = serde_json::Map::new();
    body.insert("first_name".into(), json!(first));
    body.insert("last_name".into(), json!(last));

    let mut put = |k: &str, v: &Option<String>| {
        if let Some(val) = v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            body.insert(k.to_string(), json!(val));
        }
    };
    put("email", &lead.email);
    put("phone", &lead.phone);
    put("company", &lead.company);

    let mut tags: Vec<String> = vec![
        "funnelswift-lead".to_string(),
        "source:funnelswift".to_string(),
    ];
    if let Some(src) = lead
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        tags.push(format!("funnelswift:{src}"));
    }
    tags.extend(lead.tags.iter().cloned());
    tags.dedup();
    body.insert("tags".into(), json!(tags));
    body.insert("source_app".into(), json!("funnelswift"));
    if !lead.fields.is_empty() {
        body.insert("custom_fields".into(), Value::Object(lead.fields.clone()));
    }
    Value::Object(body)
}

/// `GET {hub}/api/external/lists` — the tenant's CoreSwift lists (picker proxy).
pub async fn hub_lists(conn: &CoreSwiftConn) -> Result<Value, String> {
    let resp = reqwest::Client::new()
        .get(format!("{}/api/external/lists", conn.base_url))
        .header("Authorization", auth_header_value(&conn.api_key))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("CoreSwift lists request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("CoreSwift lists returned {status}: {text}"));
    }
    serde_json::from_str::<Value>(&text).map_err(|e| format!("CoreSwift lists: bad JSON: {e}"))
}

/// THE INBOUND PATH: push a captured lead into the CoreSwift hub.
///
/// `Ok(false)` = skipped (not connected, or nothing to identify the lead) — quiet by design.
/// `Err(...)` = a real failure worth logging (hub rejected / unreachable).
pub async fn push_lead_to_coreswift(
    db: &PgPool,
    tenant_id: Uuid,
    lead: LeadPayload,
) -> Result<bool, String> {
    let Some(body) = pushable_body(&lead) else {
        return Ok(false);
    };
    push_body_to_coreswift(db, tenant_id, &body).await
}

/// The hub body for a lead worth pushing — `None` when there is nothing to identify it, or the
/// address is harness residue that must never enter the hub.
fn pushable_body(lead: &LeadPayload) -> Option<Value> {
    if !lead.has_identity() {
        tracing::debug!("[coreswift] lead push skipped: no email/phone/name to identify it");
        return None;
    }
    if let Some(email) = lead.email.as_deref() {
        if is_reserved_test_email(email) {
            tracing::info!(
                "[coreswift] lead push skipped: {email} is a reserved test address (harness residue stays out of the hub)"
            );
            return None;
        }
    }
    Some(lead_body(lead))
}

/// POST an already-built hub contact body. The connection is resolved HERE, per attempt, so a
/// retried row honours a rotated key or a corrected base URL instead of the one it was queued
/// with.
///
/// `Ok(false)` = the tenant has no usable CoreSwift credential (nothing to post to).
pub async fn push_body_to_coreswift(
    db: &PgPool,
    tenant_id: Uuid,
    body: &Value,
) -> Result<bool, String> {
    let Some(conn) = resolve_conn(db, tenant_id).await? else {
        tracing::debug!(
            "[coreswift] lead push skipped: CoreSwift not connected for tenant {tenant_id}"
        );
        return Ok(false);
    };

    let resp = reqwest::Client::new()
        .post(format!("{}/api/external/contacts", conn.base_url))
        .header("Authorization", auth_header_value(&conn.api_key))
        .json(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("CoreSwift contact push failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift contact push returned {status}: {text}"));
    }
    tracing::info!(
        "[coreswift] lead pushed to hub {} (tenant {tenant_id})",
        conn.base_url
    );
    Ok(true)
}

// ───────────────────────── durable retry (2026-11-25) ─────────────────────────
//
// `spawn_lead_push` used to be fire-and-forget: a hub that answered 502 during a deploy window
// left the lead in FunnelSwift with a log line and nothing that ever tried again (measured
// 2026-11-25 00:01-00:08, six consecutive 502s). The capture was never lost, but the CRM hand-off
// was. A failed push is now queued in `coreswift_push_queue` (migration 146) and this worker
// re-posts it — the same shape as the outbound-webhook retry worker, which has run in production
// since kanban t_431faa99.

/// How often the worker looks for due rows.
const PUSH_SWEEP_INTERVAL_SECS: u64 = 60;
/// Rows claimed per sweep, so one sweep cannot sit in the request path for minutes.
const PUSH_SWEEP_BATCH: usize = 20;
/// First retry waits a minute; each later one doubles, capped at an hour.
const PUSH_BACKOFF_BASE_SECS: i64 = 60;
const PUSH_BACKOFF_MAX_SECS: i64 = 3600;

/// Exponential backoff in seconds for the attempt that just failed.
fn push_backoff_secs(attempt: i32) -> i64 {
    let factor = 1i64 << (attempt.saturating_sub(1).clamp(0, 10) as u32);
    (PUSH_BACKOFF_BASE_SECS * factor).min(PUSH_BACKOFF_MAX_SECS)
}

/// Record a push the hub refused, so it is retried instead of dropped.
async fn enqueue_push(db: &PgPool, tenant_id: Uuid, body: &Value, error: &str) {
    let reason: String = error.chars().take(500).collect();
    let inserted = sqlx::query(
        "INSERT INTO coreswift_push_queue (tenant_id, payload, last_error, next_retry_at) \
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(tenant_id)
    .bind(body)
    .bind(&reason)
    .bind(PUSH_BACKOFF_BASE_SECS as f64)
    .execute(db)
    .await;
    match inserted {
        Ok(_) => tracing::warn!(
            %tenant_id,
            "coreswift push queued for retry after: {reason}"
        ),
        // Not queued is worse than a slow retry: say so loudly, the customer's CRM is short a contact.
        Err(e) => tracing::error!(
            %tenant_id,
            "coreswift push NOT queued ({e}) — the hub refused with: {reason}"
        ),
    }
}

/// Fire-and-forget wrapper for capture handlers: the visitor's submission must never fail
/// or slow down because the hub is missing, slow or broken. A real failure is queued.
pub fn spawn_lead_push(db: PgPool, tenant_id: Uuid, lead: LeadPayload) {
    tokio::spawn(async move {
        let Some(body) = pushable_body(&lead) else {
            return;
        };
        if let Err(e) = push_body_to_coreswift(&db, tenant_id, &body).await {
            enqueue_push(&db, tenant_id, &body, &e).await;
        }
    });
}

/// Boot-time worker: retries queued pushes. Started from `main` next to the other background
/// tasks; a failure inside one sweep is logged and the loop continues.
pub fn spawn_push_retry_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(PUSH_SWEEP_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match sweep_pushes_once(&pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(retried = n, "coreswift push sweep"),
                Err(e) => tracing::warn!(error = %e, "coreswift push sweep failed"),
            }
        }
    });
}

/// Claims and re-sends at most `PUSH_SWEEP_BATCH` due rows. The claim is a `FOR UPDATE SKIP
/// LOCKED` select plus a conditional status update, so two workers (or two app instances) never
/// take the same row.
async fn sweep_pushes_once(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let mut handled = 0usize;
    while handled < PUSH_SWEEP_BATCH {
        let mut tx = pool.begin().await?;
        let claimed: Option<(Uuid, Uuid, Value, i32, i32)> = sqlx::query_as(
            "SELECT id, tenant_id, payload, attempt, max_attempts FROM coreswift_push_queue \
             WHERE status = 'pending' AND attempt < max_attempts \
               AND (next_retry_at IS NULL OR next_retry_at <= now()) \
             ORDER BY created_at ASC LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, tenant_id, payload, attempt, max_attempts)) = claimed else {
            tx.commit().await?;
            break;
        };
        let taken = sqlx::query(
            "UPDATE coreswift_push_queue SET status = 'sending', attempt = attempt + 1 \
             WHERE id = $1 AND status = 'pending'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        if taken.rows_affected() == 0 {
            continue;
        }

        let attempted = attempt + 1;
        let reason: Option<String> = match push_body_to_coreswift(pool, tenant_id, &payload).await {
            Ok(true) => None,
            Ok(false) => Some("CoreSwift is not connected for this tenant".to_string()),
            Err(e) => Some(e),
        };

        match reason {
            None => {
                sqlx::query(
                    "UPDATE coreswift_push_queue SET status = 'sent', sent_at = now(), \
                     last_error = NULL WHERE id = $1",
                )
                .bind(id)
                .execute(pool)
                .await?;
            }
            Some(reason) => {
                let exhausted = attempted >= max_attempts;
                let delay = push_backoff_secs(attempted);
                sqlx::query(
                    "UPDATE coreswift_push_queue \
                     SET status = CASE WHEN $2 THEN 'failed' ELSE 'pending' END, \
                         last_error = $3, next_retry_at = now() + make_interval(secs => $4) \
                     WHERE id = $1",
                )
                .bind(id)
                .bind(exhausted)
                .bind(reason.chars().take(500).collect::<String>())
                .bind(delay as f64)
                .execute(pool)
                .await?;
                if exhausted {
                    tracing::error!(
                        %tenant_id,
                        "coreswift push gave up after {attempted} attempts: {reason}"
                    );
                } else {
                    tracing::warn!(
                        %tenant_id,
                        "coreswift push retry {attempted} failed, next in {delay}s: {reason}"
                    );
                }
            }
        }
        handled += 1;
    }
    Ok(handled)
}
