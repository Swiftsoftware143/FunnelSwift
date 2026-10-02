//! THE ONE reader of `affiliate_products.source_app`.
//!
//! WHY THIS FILE EXISTS (kanban t_68e92c8e, measured 2026-10-02)
//!
//! Two handlers credited a conversion to a product with their own copy of
//!
//! ```text
//! SELECT id FROM affiliate_products WHERE source_app = $1 AND is_active = true LIMIT 1
//! ```
//!
//! (`handlers/affiliate_tracking_handler.rs:198` and `handlers/cross_app_webhook_handler.rs:185`).
//! `LIMIT 1` with **no `ORDER BY`** leaves the row to the planner, so when a key resolved to more
//! than one active product the stored `affiliate_commissions.product_id` was not reproducible and
//! a second, independent copy of the rule could drift from the first.
//!
//! MEASURED BEFORE THIS FILE (2026-10-02, shipped binary against a LIVE and a from-zero database):
//!
//! * every key the fleet actually sends resolves exactly ONE active product —
//!   `workflowswift`, `coreswift`, `incentiveswift`, `adaswift`, `missedcallrespondr`
//!   (the literals each sibling app compiles in, recorded in 055's header);
//! * `funnelswift` resolves TWO — `FunnelSwift Capture Free` and `FunnelSwift Kinetic Free` — and
//!   that is the product's design, not drift: a FunnelSwift affiliate product is one row per FREE
//!   PLAN (migration 067 mints `'FunnelSwift ' || plan.name` with `plan_id` set), so the key is
//!   genuinely multi-row;
//! * 043's `missedcall` spelling is gone from both databases (074 retires it on a from-zero build),
//!   so no system tag routes to more than one active product any more.
//!
//! THE DECISION (the card's arm (a) ENFORCE, completed plan-aware as its own note requires)
//!
//! 1. Migration 075 enforces the invariant in the database rather than trusting the migration that
//!    happened to create the rows: a partial **unique index**
//!    `uq_affiliate_products_active_key ON affiliate_products (source_app) WHERE is_active AND
//!    source_app IS NOT NULL AND plan_id IS NULL`. It is partial ON PURPOSE so it is plan-aware — a
//!    platform-wide free product (one per caller key, `plan_id` NULL) is unique, while FunnelSwift's
//!    per-free-plan rows (`plan_id` set) are keyed by their plan and may share the `funnelswift` key.
//!    A blanket `NOT NULL` + unique index was refused: the admin product form
//!    (`affiliate_product_handler::create_affiliate_product`) inserts a product with no `source_app`
//!    at all, and a NULL `source_app` can never satisfy `source_app = $1`, so it is not a key.
//! 2. THIS resolver is the single reader of that column, with a deterministic and plan-aware
//!    `ORDER BY`. With the index in place the order is a no-op for every caller key (one row), and it
//!    is what makes the genuinely multi-row `funnelswift` key reproducible: the plan the caller names
//!    wins, then a plan-derived row over a key-only row, then `name`, `created_at`, `id` — a total
//!    order, so the same key resolves the same product on every call.
//!
//! Whoever changes the predicate is changing the affiliate money path: keep migration 075's index and
//! this ORDER BY in step, and keep `is_active = true` (a retired product stops being attributable).

use sqlx::PgPool;
use uuid::Uuid;

/// Resolve the ONE active affiliate product a conversion for `source_app` is attributed to.
///
/// `plan_hint` is the plan (or product) name the caller reported, when it reported one; it is the
/// plan-aware tiebreak and is ignored when empty. Returns `None` when the key resolves no active
/// product — the callers keep their existing "unattributed" behaviour for that case.
pub async fn resolve_active_by_source_app(
    pool: &PgPool,
    source_app: &str,
    plan_hint: Option<&str>,
) -> Result<Option<Uuid>, sqlx::Error> {
    let hint: Option<&str> = plan_hint.map(str::trim).filter(|h| !h.is_empty());
    sqlx::query_scalar(
        "SELECT p.id \
           FROM affiliate_products p \
           LEFT JOIN plans pl ON pl.id = p.plan_id \
          WHERE p.source_app = $1 AND p.is_active = true \
          ORDER BY CASE \
                     WHEN $2::text IS NULL THEN 1 \
                     WHEN lower(p.name) = lower($2) THEN 0 \
                     WHEN lower(pl.name) = lower($2) THEN 0 \
                     WHEN lower(p.name) = lower('FunnelSwift ' || $2) THEN 1 \
                     WHEN lower(p.name) LIKE '%' || lower($2) || '%' THEN 2 \
                     WHEN lower(pl.name) LIKE '%' || lower($2) || '%' THEN 2 \
                     ELSE 3 \
                   END ASC, \
                   (p.plan_id IS NOT NULL) DESC, \
                   p.name ASC NULLS LAST, \
                   p.created_at ASC, \
                   p.id ASC \
          LIMIT 1",
    )
    .bind(source_app)
    .bind(hint)
    .fetch_optional(pool)
    .await
}
