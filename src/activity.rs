//! Tenant activity feed — the WRITER behind `GET /api/v1/dashboard/activity`.
//!
//! Before this module the route had one reader and `activity_log` had only two writers (the
//! `stage_change` arm of the lead handler and the API-key handlers), so the customer dashboard's
//! Recent Activity table could never show this app's headline event: a lead was captured. A feed
//! that reports the key rotas and never the capture is the same phantom surface the fleet already
//! retired on MissedCall (2026-11-16), so every door that writes a `leads` row now emits
//! `lead.created` through this one helper.
//!
//! Every call is BEST-EFFORT. A stranger filling a public capture form must never lose their lead
//! because its audit row could not be written, so a failure is logged and swallowed.
//!
//! `activity_log.user_id` is `varchar NOT NULL` with no FK on this app, so a door with no signed-in
//! user records [`PUBLIC_ACTOR`] rather than inventing a user id.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// The actor recorded for a row minted by a door with no signed-in user (a Kinetic card submit, a
/// funnel form, a web-to-lead post).
pub const PUBLIC_ACTOR: &str = "public";

/// Append one activity row for a tenant. Never propagates an error to the caller.
pub async fn log(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: &str,
    action: &str,
    entity_type: &str,
    entity_id: Option<&str>,
    metadata: Value,
) {
    let res = sqlx::query(
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) \
         VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6)",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(metadata)
    .execute(pool)
    .await;

    if let Err(e) = res {
        tracing::warn!(
            tenant = %tenant_id,
            action,
            error = %e,
            "activity_log write failed (best-effort — the request continues)"
        );
    }
}

/// A lead was captured into this tenant. `source` is the door's own source value
/// (`kinetic_card`, `funnel`, `web`, `affiliate`, …) so the dashboard can say where it came from.
pub async fn lead_created(
    pool: &PgPool,
    tenant_id: Uuid,
    actor: &str,
    lead_id: Uuid,
    name: &str,
    source: Option<&str>,
) {
    let id = lead_id.to_string();
    log(
        pool,
        tenant_id,
        actor,
        "lead.created",
        "lead",
        Some(&id),
        serde_json::json!({ "name": name, "source": source }),
    )
    .await;
}
