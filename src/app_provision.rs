//! Tag → free account: FunnelSwift is the CALLER.
//!
//! David, 2026-10-05: a lead can bypass the affiliate referral link and be onboarded through
//! FunnelSwift tagged with the software they are interested in, and tagging must create a FREE
//! ACCOUNT in that app — one the business can then upgrade from inside that app.
//!
//! The mapping half already existed (`tags.source_app` + `tags.plan_slug` on the seven
//! `<App> — Free` system tags, rendered by `affiliate_onboarding_handler::promotable_products`);
//! this module is the EXECUTOR. Design and the frozen per-app contract:
//! `/opt/swift/docs/tag-to-free-account-design-2026-10-06.md` §3.1–3.2.
//!
//! ONE generic client, not six bespoke handlers:
//!
//! ```text
//! POST {base_url}/api/v1/internal/provision-free-account
//! x-internal-key: <shared INTERNAL_SYNC_KEY>
//! { "source": "funnelswift", "source_tenant_id": "...",
//!   "tag": { "name": "...", "plan_slug": "..." },
//!   "contact": { "email", "first_name", "last_name", "company", "phone" },
//!   "idempotency_key": "<lead uuid>:<app slug>" }
//! ```
//!
//! Discipline, in the order it matters:
//!
//! * **Only 200/201 is a success.** `201 provisioned` and `200 already_exists` are the two accepted
//!   answers; 403 (that app's master toggle is off), 422 (bad address / no free plan) and anything
//!   else are recorded as what they are. A sibling's 2xx built from the request body cannot make a
//!   failure look like a success here, because the status written to the log is OUR reading of the
//!   real HTTP status, never a value we invented.
//! * **The row is the record.** Written `pending` BEFORE the call and updated with the outcome, so a
//!   call that never completed is visible as `pending` rather than absent. Same shape as
//!   `webhook_delivery_log`.
//! * **A tag write never fails because a sibling is down.** Every error is logged and swallowed: the
//!   operator's tag is stored, and `provisioning_log` is where the delivery is made visible.
//! * **Never mint twice.** The idempotency key is `<lead uuid>:<app slug>`, and the target app is
//!   idempotent on `LOWER(email)` itself (design §3.1 rule 3), so a re-apply answers `already_exists`.
//! * **Never on removal.** `apply_lead_tags` calls in with the ADDED names only; taking a tag off a
//!   lead performs no account action — an account is never deleted because a tag came off a lead.

use std::time::Duration;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::AppState;

/// The app whose account a tag names, and where to reach it.
///
/// The base URL is env-first with a loopback default, exactly like `main.rs` resolves
/// `CORESWIFT_URL`/`WORKFLOWSWIFT_URL` today: every app runs `network_mode=host` on this box, so a
/// missing env var must not mean "provisioning silently does nothing".
///
/// `funnelswift` returns `None` on purpose: a FunnelSwift lead's owning workspace **is** the
/// FunnelSwift account, so `Capture Free` / `Kinetic Free` have nothing to mint (design §3.2).
/// The SQL in [`provision_for_added_tags`] already excludes that slug; this arm is the second,
/// same-answer guard so a row that slipped through still cannot call FunnelSwift itself.
pub fn base_url(source_app: &str) -> Option<String> {
    let (var, default) = match source_app {
        "coreswift" => ("CORESWIFT_URL", "http://127.0.0.1:8084"),
        "workflowswift" => ("WORKFLOWSWIFT_URL", "http://127.0.0.1:8085"),
        "adaswift" => ("ADASWIFT_URL", "http://127.0.0.1:8087"),
        "incentiveswift" => ("INCENTIVESWIFT_URL", "http://127.0.0.1:8083"),
        "missedcallrespondr" => ("MISSEDCALL_URL", "http://127.0.0.1:8088"),
        "funnelswift" => return None,
        _ => return None,
    };
    let url = std::env::var(var)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string());
    Some(url.trim_end_matches('/').to_string())
}

/// The person the free account belongs to. Built from the lead row, never from the request body,
/// so the account is minted for the address the lead actually carries.
#[derive(Clone, Debug, Default)]
struct Contact {
    email: String,
    first_name: String,
    last_name: String,
    company: Option<String>,
    phone: Option<String>,
}

/// One tag application to execute.
struct ProvisionRequest {
    lead_id: Uuid,
    tenant_id: Uuid,
    tag_id: Uuid,
    tag_name: String,
    source_app: String,
    plan_slug: Option<String>,
}

/// `first` / `rest`, splitting a stored lead name the same way the app does elsewhere.
fn split_name(full: &str) -> (String, String) {
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }
    let mut parts = trimmed.split_whitespace();
    let first = parts.next().unwrap_or_default().to_string();
    let last = parts.collect::<Vec<_>>().join(" ");
    (first, last)
}

/// One line, no newlines, capped — an error field must not be able to store a flood.
fn one_line(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let squashed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if squashed.chars().count() > 500 {
        squashed.chars().take(500).collect::<String>() + "..."
    } else {
        squashed
    }
}

/// THE call site: for every tag that was just ADDED to a lead, mint a free account in the app the
/// tag names.
///
/// Called from `apply_lead_tags` with the names that were not on the lead before the request
/// (directly assigned AND rule-driven). Never called with removals.
///
/// Everything here is best-effort by design: the only failure that reaches a caller is a
/// `provisioning_log` INSERT we could not write, and even that is logged, not returned — the lead's
/// tag write has already happened and must not be undone by a sibling app being unreachable.
pub async fn provision_for_added_tags(
    state: &AppState,
    tenant_id: Uuid,
    lead_id: Uuid,
    added_tags: &[String],
) {
    if added_tags.is_empty() {
        return;
    }

    // The tags that carry an executed mapping. `is_system = true` is part of the predicate because
    // the vocabulary the picker offers is the SYSTEM tag list (tag_handler.rs `tenant_id = $1 OR
    // is_system = true`), the same predicate `apply_lead_tags` resolves names with.
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        "SELECT id, name, source_app, plan_slug FROM tags \
         WHERE name = ANY($1) AND (tenant_id = $2 OR is_system = true) \
           AND provisions_account = true \
           AND source_app IS NOT NULL \
           AND source_app <> 'funnelswift'",
    )
    .bind(added_tags)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                "[app_provision] tag lookup failed for lead {lead_id} (no account provisioned): {e}"
            );
            return;
        }
    };
    if rows.is_empty() {
        tracing::debug!(
            "[app_provision] lead {lead_id}: {} added tag(s), none provision an account",
            added_tags.len()
        );
        return;
    }

    // The contact comes from the lead row itself. `email` is NULLABLE and NULL on a real share of
    // live rows (lead_handler.rs measured 2 of 53); a lead with no usable address gets a
    // `refused`-shaped attempt recorded, never a fabricated placeholder account (design §3.1 rule 5).
    let lead = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
        "SELECT COALESCE(name, '') AS name, email, company, phone \
           FROM leads WHERE id = $1 AND tenant_id = $2",
    )
    .bind(lead_id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await;

    let (name, email, company, phone) = match lead {
        Ok(Some(row)) => row,
        Ok(None) => {
            tracing::warn!(
                "[app_provision] lead {lead_id} vanished before provisioning; nothing minted"
            );
            return;
        }
        Err(e) => {
            tracing::error!("[app_provision] lead read failed for {lead_id}: {e}");
            return;
        }
    };
    let (first_name, last_name) = split_name(&name);
    let contact = Contact {
        email: email.unwrap_or_default().trim().to_string(),
        first_name,
        last_name,
        company: company
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        phone: phone
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    };

    for (tag_id, tag_name, source_app, plan_slug) in rows {
        let req = ProvisionRequest {
            lead_id,
            tenant_id,
            tag_id,
            tag_name,
            source_app,
            plan_slug: plan_slug
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        };
        provision_one(state, &req, &contact).await;
    }
}

/// Execute one attempt: record it, call the target app, record what really happened.
async fn provision_one(state: &AppState, req: &ProvisionRequest, contact: &Contact) {
    let idempotency_key = format!("{}:{}", req.lead_id, req.source_app);

    // 1. The row exists BEFORE the call. A crash or a hung request therefore leaves a visible
    //    `pending` row instead of no evidence at all.
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO provisioning_log \
           (lead_id, tenant_id, tag_id, source_app, plan_slug, idempotency_key, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 'pending') RETURNING id",
    )
    .bind(req.lead_id)
    .bind(req.tenant_id)
    .bind(req.tag_id)
    .bind(&req.source_app)
    .bind(&req.plan_slug)
    .bind(&idempotency_key)
    .fetch_one(&state.pool)
    .await;

    let log_id = match inserted {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(
                "[app_provision] could not record the attempt for tag '{}' -> {}: {e}",
                req.tag_name,
                req.source_app
            );
            return;
        }
    };

    let Some(base) = base_url(&req.source_app) else {
        finish(
            state,
            log_id,
            "unknown_app",
            None,
            Some(&format!(
                "no base URL and no loopback default for app '{}'",
                req.source_app
            )),
        )
        .await;
        return;
    };

    let body = json!({
        "source": "funnelswift",
        "source_tenant_id": req.tenant_id.to_string(),
        "tag": { "name": req.tag_name, "plan_slug": req.plan_slug },
        "contact": {
            "email": contact.email,
            "first_name": contact.first_name,
            "last_name": contact.last_name,
            "company": contact.company,
            "phone": contact.phone,
        },
        "idempotency_key": idempotency_key,
    });

    let url = format!("{base}/api/v1/internal/provision-free-account");
    let resp = reqwest::Client::new()
        .post(&url)
        .header("x-internal-key", &state.internal_sync_key)
        .json(&body)
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    let (status, http_status, error_message) = match resp {
        Ok(r) => {
            let code = r.status().as_u16() as i32;
            let text = r.text().await.unwrap_or_default();
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            // The reason the sibling gave, when it named one — kept verbatim (single-lined, capped)
            // so a refusal is diagnosable from OUR log without re-running the call.
            let reason = parsed
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    parsed
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            let detail = reason.clone().or_else(|| {
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text.clone())
                }
            });

            match code {
                201 => ("provisioned", Some(code), None),
                200 => ("already_exists", Some(code), None),
                403 => ("refused", Some(code), detail.as_deref().map(one_line)),
                422 => ("invalid", Some(code), detail.as_deref().map(one_line)),
                _ => (
                    "failed",
                    Some(code),
                    Some(one_line(detail.as_deref().unwrap_or("no response body"))),
                ),
            }
        }
        Err(e) => ("failed", None, Some(one_line(&e.to_string()))),
    };

    finish(state, log_id, status, http_status, error_message.as_deref()).await;

    match status {
        "provisioned" | "already_exists" => tracing::info!(
            "[app_provision] tag '{}' -> {} free account ({status}, http {http_status:?}) for lead {}",
            req.tag_name,
            req.source_app,
            req.lead_id
        ),
        _ => tracing::warn!(
            "[app_provision] tag '{}' -> {} did NOT provision ({status}, http {http_status:?}): {}",
            req.tag_name,
            req.source_app,
            error_message.as_deref().unwrap_or("no detail")
        ),
    }
}

/// Write the outcome. Never a silent no-op: a failure to record is itself logged.
async fn finish(
    state: &AppState,
    log_id: Uuid,
    status: &str,
    http_status: Option<i32>,
    error_message: Option<&str>,
) {
    if let Err(e) = sqlx::query(
        "UPDATE provisioning_log \
            SET status = $1, http_status = $2, error_message = $3, updated_at = now() \
          WHERE id = $4",
    )
    .bind(status)
    .bind(http_status)
    .bind(error_message)
    .bind(log_id)
    .execute(&state.pool)
    .await
    {
        tracing::error!("[app_provision] could not record outcome for log row {log_id}: {e}");
    }
}

/// Read the provisioning history for a lead, newest first. Used by the probe harness and by any
/// future console view; returns raw rows so a caller can print exactly what was stored.
pub async fn history_for_lead(
    pool: &PgPool,
    lead_id: Uuid,
) -> Result<Vec<(String, String, Option<i32>, Option<String>)>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, Option<i32>, Option<String>)>(
        "SELECT source_app, status, http_status, error_message FROM provisioning_log \
          WHERE lead_id = $1 ORDER BY created_at DESC",
    )
    .bind(lead_id)
    .fetch_all(pool)
    .await
}
