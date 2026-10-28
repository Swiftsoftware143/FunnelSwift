//! Feature limits enforcement for FunnelSwift.
//! Reads plan limits from the plans table (max_cards, max_leads, etc.).
//! Falls back to feature_limits table for any custom limits defined there.
//!
//! A workspace with NO active `tenant_plan_subscriptions` row is bounded by the install's FREE
//! tier (the price-0 plan a new signup is seated on) — never left unlimited (kanban t_c5eedbd0).
//! See [`effective_plan`].
//!
//! Plan-gating is enforced through three helpers:
//!   - `enforce_feature_limit`  — numeric limits (max_cards, max_leads, ...)
//!   - `enforce_feature_flag`   — boolean flags (has_api, has_dual_routing, ...)
//!   - `enforce_action_button_limit` — per-card `max_action_buttons` limit
//!
//! Precedence, for BOTH kinds (kanban t_0a139e57 — the tenant override used to be written by the
//! admin panel and read by nothing):
//!
//!   tenant override (`tenant_settings.feature_override`)  >  plan `feature_limits` row  >  plan column
//!
//! A tenant override is the operator's one per-tenant lever (`POST
//! /api/v1/admin/tenants/:id/feature-override`); it is a VALUE even when negative, so `-1`
//! (unlimited) wins over a plan cap instead of falling through. See [`pick_limit`].

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use serde_json::Value;
use uuid::Uuid;

/// The `tenant_settings` key holding a tenant's single override slot:
/// `{"feature_key": <registry key>, "limit_value": <int>}`. Written by
/// `POST /api/v1/admin/tenants/:id/feature-override` (validated against
/// [`crate::feature_registry::spec`] at the door), read by every gate below.
pub const TENANT_OVERRIDE_KEY: &str = "feature_override";

/// The precedence rule every gate follows, as a pure function so it is UNIT-TESTED rather than
/// only observed on live rows: **tenant override > the plan's `feature_limits` row > the plan's
/// own column**. `None` at a level means "not configured there" and the next level decides;
/// `None` from all three means the key is configured nowhere, which the house rule treats as
/// allow. An override is a value even when negative — `-1` (unlimited) must beat a plan cap
/// rather than fall through to it, which is why "absent" is `None` and never `0`.
pub fn pick_limit(
    tenant_override: Option<i32>,
    plan_row: Option<i32>,
    plan_column: Option<i32>,
) -> Option<i32> {
    tenant_override.or(plan_row).or(plan_column)
}

/// The tenant override resolved for a BOOLEAN registry key: `1` grants, `0` refuses. Any other
/// stored value means "no boolean override" and falls back to the plan — the writer refuses
/// out-of-range values (`feature_registry::override_value_ok`), so this arm only guards a row
/// that was hand-written into the database.
pub fn override_flag(value: i32) -> Option<bool> {
    match value {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

/// The tenant's own override value, when its stored `feature_key` is the one being resolved.
/// ONE slot per tenant, so a key mismatch (the slot names `max_cards`, the gate asks about
/// `max_leads`) is simply "no override" and the plan decides. A non-numeric stored value is
/// likewise ignored rather than allowed to 500 a gate.
async fn tenant_override_value(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<Option<i32>> {
    let raw: Option<Value> = sqlx::query_scalar(
        "SELECT value FROM tenant_settings WHERE tenant_id = $1 AND key = 'feature_override'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(raw
        .as_ref()
        .filter(|v| v.get("feature_key").and_then(Value::as_str) == Some(feature_key))
        .and_then(|v| v.get("limit_value"))
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok()))
}

/// The `plans` column that backs a numeric limit key, when the key has one.
/// `None` for feature_limits-only keys (max_webhooks / max_portfolios /
/// max_affiliates / max_tag_groups / max_integrations), which
/// `enforce_feature_limit` resolves from the `feature_limits` table alone.
///
/// ONE allowlist: `plan_limit` and the admin feature registry both read the column through
/// this function, so a key cannot be enforced against one column while the admin panel shows
/// another (or nothing) — the drift class this table exists to kill.
pub fn limit_column(feature_key: &str) -> Option<&'static str> {
    Some(match feature_key {
        "max_cards" | "max_kinetic_cards" => "max_cards",
        "max_leads" => "max_leads",
        "max_tags" => "max_tags",
        "max_forms" => "max_forms",
        "max_custom_domains" => "max_custom_domains",
        "max_team_members" | "team_members" => "max_team_members",
        "max_qr_codes" => "max_qr_codes",
        "max_action_buttons" => "max_action_buttons",
        "max_ocr_scans" => "max_ocr_scans",
        _ => return None,
    })
}

/// A workspace's governing plan, resolved ONCE for every gate (kanban t_c5eedbd0).
///
/// `plan_id` is the plan the gates read `feature_limits` rows from; `row` is that plan serialised
/// (`to_jsonb(p)`), so a column and a jsonb flag are read out of the same bytes; `defaulted` is
/// `true` when the workspace holds NO active `tenant_plan_subscriptions` row and the install's
/// free tier was applied instead.
struct EffectivePlan {
    plan_id: Uuid,
    row: Value,
    defaulted: bool,
}

/// The workspace's own active subscription — the only plan a customer is actually paying for.
const ACTIVE_PLAN_SQL: &str = "SELECT p.id, to_jsonb(p) FROM plans p \
     JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id \
     WHERE tps.tenant_id = $1 AND tps.status = 'active' \
     ORDER BY tps.start_date DESC LIMIT 1";

/// The install's FREE tier for THIS workspace's product side — the fail-CLOSED fallback for a
/// workspace with no active subscription.
///
/// `plans.price` is `double precision NOT NULL DEFAULT 0` and `plans.side` is
/// `character varying(20) NOT NULL DEFAULT 'main'`, so unlike a slug this predicate resolves on
/// any install — it is the same pair `plan_resolver::assign_free_plan` seats every signup on, so a
/// plan-less workspace is bounded exactly as the workspace a new signup gets. The side follows the
/// workspace's own product: one that already owns a kinetic card is the Kinetic product, anything
/// else is the Capture product this app's own signup page serves (`main`, the same default the two
/// signup handlers use when the request carries no product marker).
const FREE_PLAN_SQL: &str = "SELECT p.id, to_jsonb(p) FROM plans p WHERE p.price = 0 \
     ORDER BY (p.side = CASE WHEN EXISTS \
         (SELECT 1 FROM kinetic_cards k WHERE k.tenant_id = $1) THEN 'kinetic' ELSE 'main' END) DESC, \
       (p.side = 'both') DESC, p.slug LIMIT 1";

/// Resolve the plan every gate below reads, failing CLOSED to the free tier.
///
/// BEFORE t_c5eedbd0 a workspace with no active plan was treated as UNLIMITED by every gate — a
/// lapsed, deleted or never-written subscription row bought the whole paid product. `None` is now
/// returned only when this install carries NO price-0 plan at all (nothing exists to bound the
/// tenant by); callers keep the house rule for that case and log it loudly.
async fn effective_plan(state: &AppState, tenant_id: Uuid) -> AppResult<Option<EffectivePlan>> {
    if let Some((plan_id, row)) = sqlx::query_as::<_, (Uuid, Value)>(ACTIVE_PLAN_SQL)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?
    {
        return Ok(Some(EffectivePlan {
            plan_id,
            row,
            defaulted: false,
        }));
    }

    let free = sqlx::query_as::<_, (Uuid, Value)>(FREE_PLAN_SQL)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?;

    let Some((plan_id, row)) = free else {
        tracing::error!(
            tenant = %tenant_id,
            "gates: workspace has NO active plan and this install carries NO price-0 plan — \
             nothing can bound it, so it is allowed (create a free plan in Admin > Plans)"
        );
        return Ok(None);
    };

    warn_defaulted_once(tenant_id, &row);
    Ok(Some(EffectivePlan {
        plan_id,
        row,
        defaulted: true,
    }))
}

/// The tier the GATES enforce for this workspace, as `(name, defaulted)`.
///
/// Published by `auth::handlers::me` so the customer's own "Your Plan" card can name the level it
/// is actually being held to. Measured defect, 2026-10-28: after t_c5eedbd0 made a plan-less
/// workspace fail CLOSED to the install's free tier, the dashboard still told the customer "its
/// limits are not set" — the panel and the gate disagreed about the same workspace, which is the
/// divergence `features.rs` exists to prevent (missedcallrespondr's Tenants list already publishes
/// `plan_effective_name` + `plan_is_default` for exactly this state).
///
/// `None` means nothing could be resolved (this install carries no price-0 plan); the caller then
/// says nothing about a level rather than inventing one.
pub async fn effective_plan_label(
    state: &AppState,
    tenant_id: Uuid,
) -> AppResult<Option<(String, bool)>> {
    Ok(effective_plan(state, tenant_id).await?.and_then(|e| {
        e.row
            .get("name")
            .and_then(Value::as_str)
            .map(|name| (name.to_string(), e.defaulted))
    }))
}

/// One WARN per plan-less workspace per process, not one per gate evaluation: the fallback is
/// correct but the operator should still see which workspaces have no subscription.
fn warn_defaulted_once(tenant_id: Uuid, row: &Value) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static WARNED: OnceLock<Mutex<HashSet<Uuid>>> = OnceLock::new();
    let seen = WARNED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut seen = match seen.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if seen.insert(tenant_id) {
        let slug = row.get("slug").and_then(Value::as_str).unwrap_or("?");
        tracing::warn!(
            tenant = %tenant_id,
            plan = %slug,
            "gates: workspace has NO active plan — the install's free tier applies (fail-closed)"
        );
    }
}

/// Read a numeric limit column out of a serialised plan row. `None` = the key has no column, or
/// the column is NULL — both "not configured", which the house rule treats as allow.
fn plan_column_limit(row: &Value, feature_key: &str) -> Option<i32> {
    limit_column(feature_key)
        .and_then(|column| row.get(column))
        .and_then(Value::as_i64)
        .map(|n| n as i32)
}

/// Evaluate a raw numeric limit against current usage. `limit` values follow the
/// plan convention: `-1` = unlimited, `0` = feature disabled, `>0` = capped.
fn check_numeric_limit(limit: i32, usage: i64, label: &str) -> AppResult<()> {
    if limit == -1 {
        return Ok(()); // unlimited
    }
    if limit == 0 {
        return Err(AppError::UpgradeRequired(format!(
            "{} is not available on your current plan. Upgrade to access this feature.",
            label
        )));
    }
    if usage >= limit as i64 {
        return Err(AppError::UpgradeRequired(format!(
            "{} limit reached ({}/{}). Upgrade to increase your limit.",
            label, usage, limit
        )));
    }
    Ok(())
}

pub async fn enforce_feature_limit(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
    label: &str,
) -> AppResult<()> {
    match resolved_limit(state, tenant_id, feature_key).await? {
        // No plan to read at all (this install has no price-0 plan to fall back to), or the key is
        // configured nowhere on the effective plan — both "not configured", which the house rule
        // treats as allow. A plan-less workspace is NOT this case: `effective_plan` gives it the
        // install's free tier, whose own numbers decide.
        None => Ok(()),
        Some(limit) => {
            let usage = get_usage_count(state, tenant_id, feature_key).await;
            check_numeric_limit(limit, usage, label)
        }
    }
}

/// Remaining headroom under the tenant's effective numeric limit (`None` = unlimited).
///
/// The bulk IMPORT path needs the number, not a pass/fail: it has to refuse a whole file BEFORE
/// writing any of it, and say how many rows would fit. Reading it through the ONE resolver keeps a
/// number the owner sets (tenant override, `feature_limits` row, plan column) authoritative here
/// exactly as it is in [`enforce_feature_limit`].
pub async fn remaining_headroom(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<Option<i64>> {
    match resolved_limit(state, tenant_id, feature_key).await? {
        // `None` = the key is configured nowhere (house rule: allow), and `-1` is the plan
        // convention for UNLIMITED (see [`check_numeric_limit`]). Both mean "no ceiling", so an
        // import must not read -1 as "zero rows fit" — measured live 2026-10-18: the Agency plan
        // carries max_leads = -1 and the first version of this helper answered 0, which refused a
        // two-row import on the plan that advertises the feature.
        None => Ok(None),
        Some(-1) => Ok(None),
        Some(limit) => {
            let used = get_usage_count(state, tenant_id, feature_key).await;
            Ok(Some((limit as i64 - used).max(0)))
        }
    }
}

/// The effective numeric limit for the tenant, in precedence order (see [`pick_limit`]): the
/// tenant's own override FIRST, then a `feature_limits` row (the custom per-plan number the admin
/// panel writes), then the plan's own column through [`plan_limit`]. `None` = the key is
/// configured nowhere, which the house rule treats as allow. ONE resolver, so a number the owner
/// sets — for the tenant, for the plan's row, or on the plan's column — is the number every limit
/// gate reads (including `enforce_action_button_limit`, which used to read the column only and
/// would have made the panel's number for that key inert).
async fn resolved_limit(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<Option<i32>> {
    // The tenant override is the highest-precedence lever and is honoured even when the tenant has
    // no active plan — an explicit per-tenant decision is the operator's strongest statement about
    // that tenant. When it decides, the plan reads are skipped: a tenant without an override pays
    // exactly what it paid before the override existed.
    if let Some(value) = tenant_override_value(state, tenant_id, feature_key).await? {
        return Ok(Some(value));
    }

    // ONE resolver, so a workspace with no active subscription is bounded by the install's free
    // tier at BOTH levels (`feature_limits` rows and the plan's own column) — kanban t_c5eedbd0.
    let Some(plan) = effective_plan(state, tenant_id).await? else {
        return Ok(None);
    };

    let plan_row: Option<i32> = sqlx::query_scalar(
        "SELECT fl.limit_value FROM feature_limits fl WHERE fl.plan_id = $1 AND fl.feature_key = $2",
    )
    .bind(plan.plan_id)
    .bind(feature_key)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    Ok(pick_limit(
        None,
        plan_row,
        plan_column_limit(&plan.row, feature_key),
    ))
}

/// Map a `has_*` column name to the corresponding `features` jsonb key.
/// Returns `None` for unknown flags (no jsonb override).
pub fn flag_jsonb_key(feature_key: &str) -> Option<&'static str> {
    match feature_key {
        "has_webhooks" => Some("webhooks"),
        "has_api" => Some("api_access"),
        "has_dual_routing" => Some("dual_routing"),
        "has_mini_funnels" => Some("mini_funnels"),
        "has_card_gating" => Some("card_gating"),
        "has_remove_branding" => Some("remove_branding"),
        "has_white_label" => Some("white_label"),
        "has_multi_tenant" => Some("multi_tenant"),
        "has_analytics" => Some("analytics"),
        "has_import_export" => Some("import_export"),
        // Premium themes/templates is a jsonb-ONLY flag (no `has_*` column). Registering it here
        // lets `plan_row_flag` and the admin feature registry resolve it through the same rule as
        // every other boolean instead of special-casing it.
        "premium_themes" => Some("premium_themes"),
        _ => None,
    }
}

/// Resolve a boolean plan flag from a serialised `plans` row (as produced by `to_jsonb(p)`).
///
/// Single source of truth for the flag rule, shared by `enforce_feature_flag` (per tenant) and
/// the admin feature registry (per plan): a PRESENT `features` jsonb key wins — `true` granted,
/// `false` refused; otherwise the `has_*` column. `None` means the row carries neither, i.e.
/// the plan configures nothing (callers decide what absence means: the gate refuses).
pub fn plan_row_flag(row: &serde_json::Value, feature_key: &str) -> Option<bool> {
    if let Some(alias) = flag_jsonb_key(feature_key) {
        if let Some(v) = row
            .get("features")
            .and_then(|f| f.get(alias))
            .and_then(|v| v.as_bool())
        {
            return Some(v);
        }
    }
    row.get(feature_key).and_then(|v| v.as_bool())
}

/// Enforce a boolean plan flag (e.g. `has_dual_routing`).
///
/// Single source of truth, in precedence order: the tenant's own override FIRST (see [`pick_limit`]
/// — `1` grants, `0` refuses), then the `features` jsonb key when present, else the `has_*` column
/// on the active plan. `true` -> Ok; `false` -> UpgradeRequired.
/// A tenant with no active plan is bounded by the install's FREE tier — the same plan a new signup
/// is seated on (kanban t_c5eedbd0) — unless the operator stored an override for this tenant,
/// which is honoured precisely because it is the only lever that speaks about this tenant alone.
pub async fn enforce_feature_flag(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
    label: &str,
) -> AppResult<()> {
    if let Some(raw) = tenant_override_value(state, tenant_id, feature_key).await? {
        if let Some(on) = override_flag(raw) {
            return if on {
                Ok(())
            } else {
                Err(AppError::UpgradeRequired(format!(
                    "{} is not available on your current plan. Upgrade to access this feature.",
                    label
                )))
            };
        }
    }

    // An install carrying no price-0 plan at all has nothing to bound the tenant by: the house
    // rule (allow) stays, and `effective_plan` has already logged the ERROR.
    let Some(plan) = effective_plan(state, tenant_id).await? else {
        return Ok(());
    };

    if plan_row_flag(&plan.row, feature_key).unwrap_or(false) {
        Ok(())
    } else {
        Err(AppError::UpgradeRequired(format!(
            "{} is not available on your current plan. Upgrade to access this feature.",
            label
        )))
    }
}

/// Enforce `max_action_buttons` — a per-card limit (counts CTA buttons on `card_id`).
pub async fn enforce_action_button_limit(
    state: &AppState,
    tenant_id: Uuid,
    card_id: Uuid,
) -> AppResult<()> {
    // Through the SAME resolver as every other limit, so the `feature_limits` row the admin panel
    // writes for `max_action_buttons` actually moves this gate (it used to read the column only).
    let limit = resolved_limit(state, tenant_id, "max_action_buttons").await?;
    let Some(limit) = limit else {
        return Ok(());
    };
    if limit == -1 {
        return Ok(()); // unlimited
    }
    if limit == 0 {
        return Err(AppError::UpgradeRequired(
            "Action buttons are not available on your current plan. Upgrade to add more.".into(),
        ));
    }
    let usage: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_buttons WHERE card_id = $1")
        .bind(card_id)
        .fetch_one(&state.pool)
        .await?;
    if usage >= limit as i64 {
        return Err(AppError::UpgradeRequired(format!(
            "Action buttons limit reached ({}/{}). Upgrade to increase your limit.",
            usage, limit
        )));
    }
    Ok(())
}

async fn get_usage_count(state: &AppState, tenant_id: Uuid, feature_key: &str) -> i64 {
    match feature_key {
        "max_leads" => sqlx::query_scalar("SELECT COUNT(*) FROM leads WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0),
        "max_tags" => sqlx::query_scalar(
            "SELECT COUNT(*) FROM tags WHERE tenant_id = $1 AND is_system = false",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0),
        "max_affiliates" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM affiliates WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "max_cards" | "max_kinetic_cards" => sqlx::query_scalar(
            // `is_template` is NOT a column on kinetic_cards (see \d kinetic_cards), so the
            // old predicate made this query ERROR on every call and `.unwrap_or(0)` returned 0.
            // Effect: enforce_feature_limit("max_cards") always saw 0 cards (so the plan card
            // limit was never enforced) and the dashboard usage counter always read 0.
            "SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0),
        "max_forms" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM web_to_lead_configs WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "max_qr_codes" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_qr_codes WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "max_webhooks" => sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0),
        // `max_api_keys` was RETIRED with the credential surface (kanban t_5a3c2d9c /
        // decision t_538505de ARM b): its only caller, api_key_handler::create_api_key, is
        // deleted, so the key had zero callers. t_726416be then removed the key from
        // feature_registry::SPECS and deleted its two rows (migration 090), so this arm is gone
        // rather than left as a literal naming a key nothing can call. The `api_keys` TABLE was
        // dropped by the same migration, and `plans.has_api` is plan/billing DATA left in place
        // (whether a paid plan still SELLS "API access" is a pricing call, not this crate's).
        "max_portfolios" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_companies WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "max_tag_groups" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM tag_groups WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        // RETIRED (kanban t_0aaf0bc5): `max_routing_targets` counted `target_software`, the resource
        // migration 088 drops. Its authored values are duplicated by the surviving `max_webhooks` on
        // the same plans (kinetic-free 5/5, kinetic-pro -1/-1), so nothing is lost by the retire.
        //
        // `max_integrations` counts the Integration Center's connections (`provider_keys`) — the rows
        // the gate in `provider_keys_handler::upsert_provider_key` refuses to grow past the cap. It
        // used to count `target_software`, which is the wrong table for a key named after the
        // Integrations screen the tenant actually uses.
        "max_integrations" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM provider_keys WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "team_members" | "max_team_members" => sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE tenant_id = $1 AND is_active = true",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0),
        "max_custom_domains" | "max_domains" => sqlx::query_scalar(
            "SELECT COUNT(*) FROM tenant_settings WHERE tenant_id = $1 AND key = 'custom_domain' AND value IS NOT NULL",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0),
        // `max_ocr_scans` was a plan column `plan_limit` reads but this match had
        // no arm for, so it fell through to `_ => 0` and `enforce_feature_limit`
        // compared `0 >= limit` — never true, so the sold metered feature was
        // never enforced. Rows are written by `handlers::ocr` (migration 045).
        "max_ocr_scans" => sqlx::query_scalar("SELECT COUNT(*) FROM ocr_scans WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0),
        _ => 0i64,
    }
}

/// Get current tenant's usage counts for plan gating (dashboard display)
pub async fn get_usage_json(state: &AppState, tenant_id: Uuid) -> serde_json::Value {
    let cards = get_usage_count(state, tenant_id, "max_cards").await;
    let leads = get_usage_count(state, tenant_id, "max_leads").await;
    let tags = get_usage_count(state, tenant_id, "max_tags").await;
    let forms = get_usage_count(state, tenant_id, "max_forms").await;
    let domains = get_usage_count(state, tenant_id, "max_custom_domains").await;
    let team = get_usage_count(state, tenant_id, "max_team_members").await;
    // `max_qr_codes` already had a counting arm (the plan GATE enforces it); it was simply not in
    // this payload, so the dashboard's QR-codes counter had no figure to put beside its allowance
    // and rendered the allowance alone ("QR codes 5") while every other counter read "used / limit".
    // One more counter on the same read, so the dashboard and the gate still come from one source.
    let qr_codes = get_usage_count(state, tenant_id, "max_qr_codes").await;
    // CARD-SCAN COUNTER (2026-10-24, cron `default`). `max_ocr_scans` is the only SOLD metered
    // feature whose usage was invisible to the customer: the gate enforces it on every upload
    // (`handlers::ocr` -> `enforce_feature_limit(..., "max_ocr_scans", ...)`) and the counting arm
    // below already existed, but this payload never carried the figure, so the plan strip could
    // print the allowance alone ("Card scans 10") and a customer could not see how much of the
    // allowance the Card Scanner had used. Same shape as the QR-codes fix above: one more counter
    // on the same read, so what the strip shows and what the gate compares come from one pair.
    let ocr_scans = get_usage_count(state, tenant_id, "max_ocr_scans").await;

    serde_json::json!({
        "cards": cards,
        "leads": leads,
        "tags": tags,
        "forms": forms,
        "domains": domains,
        "team": team,
        "qr_codes": qr_codes,
        "ocr_scans": ocr_scans
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Premium card themes + templates (`premium_themes` plan feature)
// ─────────────────────────────────────────────────────────────────────────────
// Gating rule (single source of truth for the catalogue endpoints AND the
// create/update card handlers):
//   * free themes     — midnight, ocean, rose
//   * free templates  — the `bio_*` family
//   * everything else — requires `premium_themes` on `plans.features`
// Semantics, matching `enforce_feature_flag`: the effective plan (the active subscription, else
// the install's free tier) decides — key absent/FALSE -> locked (402 UpgradeRequired);
// key TRUE -> allow; this install carries no price-0 plan at all -> allow (nothing bounds it).

/// jsonb key on `plans.features` that grants premium themes and templates.
pub const PREMIUM_THEMES_KEY: &str = "premium_themes";

/// Themes every plan may use, regardless of `premium_themes`.
pub const FREE_THEMES: &[&str] = &["midnight", "ocean", "rose"];

/// Namespace prefixes used by the template catalogue in
/// `handlers::theme_endpoint`. Card *types* (`default`, `business_card`,
/// `mini_page`, `mini_funnel`, `hero`, `thank_you`, ...) are not catalogue ids
/// and are therefore never gated by this module.
const TEMPLATE_NAMESPACES: &[&str] = &[
    "biz_", "bio_", "page_", "funnel_", "hero_", "starter_", "blank_",
];
/// Catalogue ids that carry no namespace prefix.
const TEMPLATE_STANDALONE_IDS: &[&str] = &["realestate_showcase", "creator_hub", "ecom_boutique"];

/// `"free"` or `"premium"` for a theme id.
pub fn theme_tier(theme_id: &str) -> &'static str {
    if FREE_THEMES.contains(&theme_id) {
        "free"
    } else {
        "premium"
    }
}

/// `"free"` or `"premium"` for a template id (`bio_*` templates are free).
pub fn template_tier(template_id: &str) -> &'static str {
    if template_id.starts_with("bio_") {
        "free"
    } else {
        "premium"
    }
}

/// True when `id` looks like a template-catalogue id (rather than a card type).
pub fn is_catalogue_template(id: &str) -> bool {
    TEMPLATE_NAMESPACES.iter().any(|p| id.starts_with(p)) || TEMPLATE_STANDALONE_IDS.contains(&id)
}

/// Does the workspace's effective plan grant `premium_themes`?
/// Resolved through [`plan_row_flag`] (the same rule every other boolean uses) on the plan
/// [`effective_plan`] resolves — so a workspace with no active subscription is answered by the
/// install's FREE tier, not granted premium themes by default (kanban t_c5eedbd0).
/// `true` only when the plan literally carries the key as `true`; `false` when it is absent,
/// `null` or `false`, and when this install carries no price-0 plan to fall back to (nothing
/// bounds the tenant, so the house rule stays).
pub async fn premium_themes_granted(state: &AppState, tenant_id: Uuid) -> AppResult<bool> {
    let Some(plan) = effective_plan(state, tenant_id).await? else {
        return Ok(true);
    };
    Ok(plan_row_flag(&plan.row, PREMIUM_THEMES_KEY).unwrap_or(false))
}

/// Reject a premium `theme` with 402 when the caller's plan lacks
/// `premium_themes`. Free/empty/unknown-to-catalogue values pass.
pub async fn enforce_theme_access(
    state: &AppState,
    tenant_id: Uuid,
    theme: Option<&str>,
) -> AppResult<()> {
    let Some(theme) = theme.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    if theme_tier(theme) == "free" {
        return Ok(());
    }
    if premium_themes_granted(state, tenant_id).await? {
        return Ok(());
    }
    Err(AppError::UpgradeRequired(format!(
        "The \"{}\" theme is a premium theme. Upgrade to Kinetic Pro to unlock premium themes.",
        theme
    )))
}

/// Reject a premium catalogue `template` with 402 when the caller's plan lacks
/// `premium_themes`. Free (`bio_*`) templates and non-catalogue values (card
/// types) pass untouched.
pub async fn enforce_template_access(
    state: &AppState,
    tenant_id: Uuid,
    template: Option<&str>,
) -> AppResult<()> {
    let Some(template) = template.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    if !is_catalogue_template(template) || template_tier(template) == "free" {
        return Ok(());
    }
    if premium_themes_granted(state, tenant_id).await? {
        return Ok(());
    }
    Err(AppError::UpgradeRequired(format!(
        "The \"{}\" template is a premium template. Upgrade to Kinetic Pro to unlock premium templates.",
        template
    )))
}

// The two resolution rules every gate and the admin registry SHARE. They are pure functions, so
// the precedence they encode is locked here rather than only observed on live data (kanban
// t_35acff73: `enforce_feature_flag` and the registry both call `plan_row_flag`, and
// `enforce_feature_limit` / `enforce_action_button_limit` both call `resolved_limit`).
#[cfg(test)]
mod resolution_rules {
    use super::*;

    #[test]
    fn a_present_jsonb_key_beats_the_column_both_ways() {
        // jsonb false wins over column true (this is the "panel says no" arm)
        let row = serde_json::json!({"features": {"webhooks": false}, "has_webhooks": true});
        assert_eq!(plan_row_flag(&row, "has_webhooks"), Some(false));
        // jsonb true wins over column false
        let row = serde_json::json!({"features": {"webhooks": true}, "has_webhooks": false});
        assert_eq!(plan_row_flag(&row, "has_webhooks"), Some(true));
        // absent jsonb key falls back to the column
        let row = serde_json::json!({"has_webhooks": true});
        assert_eq!(plan_row_flag(&row, "has_webhooks"), Some(true));
        // neither: the plan configures nothing (the gate refuses)
        assert_eq!(plan_row_flag(&serde_json::json!({}), "has_webhooks"), None);
        // a non-bool jsonb value is not a grant either — the column still decides
        let row = serde_json::json!({"features": {"webhooks": "true"}, "has_webhooks": false});
        assert_eq!(plan_row_flag(&row, "has_webhooks"), Some(false));
    }

    #[test]
    fn premium_themes_is_resolved_through_the_same_rule() {
        assert_eq!(
            plan_row_flag(
                &serde_json::json!({"features": {"premium_themes": true}}),
                "premium_themes"
            ),
            Some(true)
        );
        assert_eq!(
            plan_row_flag(&serde_json::json!({"features": {}}), "premium_themes"),
            None
        );
    }

    #[test]
    fn the_limit_column_allowlist_is_the_gates_own() {
        assert_eq!(limit_column("max_kinetic_cards"), Some("max_cards"));
        assert_eq!(limit_column("team_members"), Some("max_team_members"));
        assert_eq!(limit_column("max_ocr_scans"), Some("max_ocr_scans"));
        // feature_limits-only keys have no column: absence must stay "not configured"
        assert_eq!(limit_column("max_webhooks"), None);
        assert_eq!(limit_column("max_affiliates"), None);
    }

    /// kanban t_c5eedbd0: a plan row (the active subscription OR the free-tier fallback) is the
    /// only place a numeric cap comes from, so the reader must give the same answers for both.
    #[test]
    fn a_numeric_cap_is_read_out_of_the_serialised_plan_row() {
        let row = serde_json::json!({
            "max_cards": 0,
            "max_leads": 100,
            "max_forms": null
        });
        // the capture-free free tier really does cap cards at 0 — the value that now refuses a
        // plan-less workspace instead of the old fail-open allowance
        assert_eq!(plan_column_limit(&row, "max_cards"), Some(0));
        assert_eq!(plan_column_limit(&row, "max_kinetic_cards"), Some(0));
        assert_eq!(plan_column_limit(&row, "max_leads"), Some(100));
        // NULL means "not configured", never 0
        assert_eq!(plan_column_limit(&row, "max_forms"), None);
        // absent column, and a key with no column at all
        assert_eq!(plan_column_limit(&row, "max_qr_codes"), None);
        assert_eq!(plan_column_limit(&row, "max_webhooks"), None);
    }

    // ── the per-tenant override precedence (kanban t_0a139e57) ────────────────────────────────
    // The rule is `tenant override > plan feature_limits row > plan column`, and it is locked HERE
    // so a future edit to `resolved_limit` cannot quietly reorder it.

    #[test]
    fn a_tenant_override_beats_the_plan_row_and_the_plan_column() {
        assert_eq!(pick_limit(Some(1), Some(500), Some(900)), Some(1));
        assert_eq!(pick_limit(Some(1), None, Some(900)), Some(1));
        assert_eq!(pick_limit(Some(1), Some(500), None), Some(1));
        // the plan's row still beats its own column
        assert_eq!(pick_limit(None, Some(500), Some(900)), Some(500));
        assert_eq!(pick_limit(None, None, Some(900)), Some(900));
        // configured nowhere -> not configured (the house rule treats that as allow)
        assert_eq!(pick_limit(None, None, None), None);
    }

    #[test]
    fn an_override_of_negative_one_is_a_value_not_an_absence() {
        // -1 means unlimited and must WIN over a plan cap, never fall through to it
        assert_eq!(pick_limit(Some(-1), Some(5), Some(5)), Some(-1));
        // and 0 means "disabled for this tenant", not "no override"
        assert_eq!(pick_limit(Some(0), Some(-1), Some(-1)), Some(0));
    }

    #[test]
    fn a_boolean_override_maps_one_to_granted_and_zero_to_refused() {
        assert_eq!(override_flag(1), Some(true));
        assert_eq!(override_flag(0), Some(false));
        // out of range = "no boolean override" so a hand-written row can never grant/refuse at
        // random; the writer refuses these values outright (override_value_ok).
        assert_eq!(override_flag(-1), None);
        assert_eq!(override_flag(7), None);
    }
}
