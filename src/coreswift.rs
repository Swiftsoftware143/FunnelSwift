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
    if !lead.has_identity() {
        tracing::debug!("[coreswift] lead push skipped: no email/phone/name to identify it");
        return Ok(false);
    }
    let Some(conn) = resolve_conn(db, tenant_id).await? else {
        tracing::debug!(
            "[coreswift] lead push skipped: CoreSwift not connected for tenant {tenant_id}"
        );
        return Ok(false);
    };

    let body = lead_body(&lead);
    let resp = reqwest::Client::new()
        .post(format!("{}/api/external/contacts", conn.base_url))
        .header("Authorization", auth_header_value(&conn.api_key))
        .json(&body)
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

/// Fire-and-forget wrapper for capture handlers: the visitor's submission must never fail
/// or slow down because the hub is missing, slow or broken.
pub fn spawn_lead_push(db: PgPool, tenant_id: Uuid, lead: LeadPayload) {
    tokio::spawn(async move {
        match push_lead_to_coreswift(&db, tenant_id, lead).await {
            Ok(true) => {}
            Ok(false) => {}
            Err(e) => tracing::warn!("[coreswift] lead push failed (capture kept locally): {e}"),
        }
    });
}
