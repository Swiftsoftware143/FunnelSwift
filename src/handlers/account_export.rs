//! `GET /api/v1/account/export` — the customer's OWN copy of their data.
//!
//! WHY THIS EXISTS
//!
//! Every other door in this app hands the customer one screen's worth of rows. A workspace that
//! wants to leave, move or audit its own account had no way to take its data with it: the only
//! exports were per-screen (leads CSV, members CSV). This is the whole-account door, and it is
//! tenant-scoped by the SESSION's claim — there is no id in the path to spoof, so one workspace can
//! never read another's rows through it.
//!
//! SHAPE
//!
//! One JSON document, one key per collection (`account`, `users`, `leads`, …). Postgres builds each
//! collection with `json_agg` so column types never have to be re-declared in Rust, and the columns
//! are listed EXPLICITLY per table: a `SELECT *` here would leak `users.password_hash`, the sealed
//! API-key columns and a webhook's signing secret into a downloadable file. Those three classes are
//! deliberately absent and the payload says so in `excluded`.
//!
//! The download is bounded by nothing but the account's own size — it is the customer's data, and a
//! truncated copy would be a lie. It reads only; nothing is written.

use axum::{
    extract::State,
    http::header::{self, HeaderValue},
    response::{IntoResponse, Response},
};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `(key in the document, SQL returning a JSON array of the caller's rows)`.
///
/// Each query is wrapped as `SELECT coalesce(json_agg(x), '[]'::json)::text FROM (<inner>) x`, so
/// the column list lives in ONE place per table and never needs a Rust mirror type.
const COLLECTIONS: &[(&str, &str)] = &[
    (
        "account",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, slug, email, status, affiliate_code, created_at, updated_at
               FROM tenants WHERE id = $1) x",
    ),
    (
        "plan",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT s.id, p.name AS plan_name, p.slug AS plan_slug, p.price, p.annual_price,
                    s.billing_cycle, s.status, s.start_date, s.end_date, s.created_at
               FROM tenant_plan_subscriptions s
               LEFT JOIN plans p ON p.id = s.plan_id
              WHERE s.tenant_id = $1) x",
    ),
    (
        "users",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, email, name, role, is_active, first_name, last_name, username, company,
                    avatar_url, email_verified, last_login_at, closed_at, created_at
               FROM users WHERE tenant_id = $1) x",
    ),
    (
        "workspaces",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, is_default, created_at FROM workspaces WHERE tenant_id = $1) x",
    ),
    (
        "leads",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, email, phone, company, source, status, stage, score, tags,
                    custom_fields, notes, assigned_to, created_at, updated_at
               FROM leads WHERE tenant_id = $1) x",
    ),
    (
        "tags",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, color, group_id, is_system, source_app, provisions_account, created_at
               FROM tags WHERE tenant_id = $1) x",
    ),
    (
        "tag_groups",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, is_collapsible, sort_order, created_at
               FROM tag_groups WHERE tenant_id = $1) x",
    ),
    (
        "funnels",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, slug, headline, subheadline, cta_label, thank_you, is_active,
                    ask_phone, created_at
               FROM funnels WHERE tenant_id = $1) x",
    ),
    (
        "campaigns",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, campaign_type, description, status, created_at
               FROM campaigns WHERE tenant_id = $1) x",
    ),
    (
        "kinetic_cards",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, user_id, slug, title, bio, tagline, is_active, created_at
               FROM kinetic_cards WHERE tenant_id = $1) x",
    ),
    (
        "webhooks",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, url, events, is_active, created_at
               FROM webhooks WHERE tenant_id = $1) x",
    ),
    (
        "api_keys",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, name, key_prefix, created_at, last_used_at, revoked_at
               FROM api_keys WHERE tenant_id = $1) x",
    ),
    (
        "provider_keys",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, provider, base_url, scope, is_active, created_at
               FROM provider_keys WHERE tenant_id = $1) x",
    ),
    (
        "support_requests",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, subject, body, status, reply, replied_at, created_at
               FROM support_requests WHERE tenant_id = $1) x",
    ),
    (
        "plan_requests",
        "SELECT coalesce(json_agg(x), '[]'::json)::text FROM (
             SELECT id, plan_slug, plan_name, note, status, reply, replied_at, billing_cycle,
                    created_at
               FROM plan_requests WHERE tenant_id = $1) x",
    ),
];

/// The one thing a customer must be told about their own copy: what is NOT in it, and why.
const EXCLUDED: &str = "Nothing that could be replayed to sign in as somebody is included: \
     stored password hashes, API-key secrets (the key PREFIX and its dates are here), provider \
     credentials and webhook signing secrets are never exported.";

/// `GET /api/v1/account/export`
pub async fn export_account(auth: AuthUser, State(state): State<AppState>) -> AppResult<Response> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let mut out = Map::new();
    let mut counts = Map::new();

    for (key, sql) in COLLECTIONS {
        let raw: String = sqlx::query_scalar(sql)
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?;
        let rows: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!([]));
        counts.insert(
            (*key).to_string(),
            json!(rows.as_array().map(|a| a.len()).unwrap_or(0)),
        );
        out.insert((*key).to_string(), rows);
    }

    out.insert("counts".to_string(), Value::Object(counts));
    out.insert("excluded".to_string(), json!(EXCLUDED));
    out.insert(
        "exported_at".to_string(),
        json!(chrono::Utc::now().to_rfc3339()),
    );
    out.insert("format".to_string(), json!("funnelswift-account-export/1"));

    let body = serde_json::to_string_pretty(&Value::Object(out))
        .map_err(|e| AppError::Internal(format!("export encode error: {e}")))?;

    let filename = format!(
        "funnelswift-account-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .map_err(|e| AppError::Internal(format!("export header error: {e}")))?;

    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response())
}
