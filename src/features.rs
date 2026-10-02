//! Feature limits enforcement for FunnelSwift.
//! Reads plan limits from the plans table (max_cards, max_leads, etc.).
//! Falls back to feature_limits table for any custom limits defined there.
//!
//! Plan-gating is enforced through three helpers:
//!   - `enforce_feature_limit`  — numeric limits (max_cards, max_leads, ...)
//!   - `enforce_feature_flag`   — boolean flags (has_api, has_dual_routing, ...)
//!   - `enforce_action_button_limit` — per-card `max_action_buttons` limit

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use uuid::Uuid;

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

/// Numeric limit for the tenant's active plan: `feature_limits` first (custom override), then
/// the plan's own column through [`limit_column`]. `Ok(None)` when the tenant has no active
/// plan, the key has neither a row nor a column, or the column is NULL — all of which mean
/// "not configured", which the house rule treats as allow.
///
/// The column is read out of the serialised plan row (`to_jsonb(p)`), so the statement stays a
/// fixed literal — the key never reaches the SQL text.
async fn plan_limit(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<Option<i32>> {
    let Some(column) = limit_column(feature_key) else {
        return Ok(None);
    };
    let row: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(row
        .and_then(|r| r.get(column).and_then(|v| v.as_i64()))
        .map(|n| n as i32))
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
        None => Ok(()), // No plan assigned, no row and no column — allow
        Some(limit) => {
            let usage = get_usage_count(state, tenant_id, feature_key).await;
            check_numeric_limit(limit, usage, label)
        }
    }
}

/// The effective numeric limit for the tenant's active plan: a `feature_limits` row (the custom
/// per-plan override the admin panel writes) FIRST, then the plan's own column through
/// [`plan_limit`]. `None` = the key is not configured anywhere, which the house rule treats as
/// allow. ONE resolver, so a number the owner sets on a plan is the number every limit gate reads
/// — including `enforce_action_button_limit`, which used to read the column only and would have
/// made the panel's number for that key inert.
async fn resolved_limit(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<Option<i32>> {
    let fl: Option<Option<i32>> = sqlx::query_scalar(
        "SELECT fl.limit_value FROM feature_limits fl
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = fl.plan_id
         WHERE tps.tenant_id = $1 AND tps.status = 'active' AND fl.feature_key = $2
         ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(feature_key)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(val) = fl.flatten() {
        return Ok(Some(val));
    }
    plan_limit(state, tenant_id, feature_key).await
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
/// Single source of truth: prefer the `features` jsonb key when present, else the
/// `has_*` column on the active plan. `true` -> Ok; `false` -> UpgradeRequired.
/// A tenant with no active plan is allowed (matching `enforce_feature_limit`'s
/// "no plan -> allow" behaviour so tenants without a subscription are not locked out).
pub async fn enforce_feature_flag(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
    label: &str,
) -> AppResult<()> {
    let row: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM plans p
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
         WHERE tps.tenant_id = $1 AND tps.status = 'active'
         ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        return Ok(()); // no active plan — allow (consistent with numeric gating)
    };

    if plan_row_flag(&row, feature_key).unwrap_or(false) {
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

    serde_json::json!({
        "cards": cards,
        "leads": leads,
        "tags": tags,
        "forms": forms,
        "domains": domains,
        "team": team
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
// Semantics, matching `enforce_feature_flag`: no active plan -> allow;
// key absent/FALSE -> locked (402 UpgradeRequired); key TRUE -> allow.

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

/// Does the tenant's active plan grant `premium_themes`?
/// `true` when there is no active plan (fail-open, consistent with the other
/// helpers here) or when the jsonb key is literally `true`; `false` when the
/// key is absent, `null`, or `false`.
pub async fn premium_themes_granted(state: &AppState, tenant_id: Uuid) -> AppResult<bool> {
    let granted: Option<bool> = sqlx::query_scalar(
        "SELECT (COALESCE(p.features->>$2, 'false') = 'true')
         FROM plans p
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
         WHERE tps.tenant_id = $1 AND tps.status = 'active'
         ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(PREMIUM_THEMES_KEY)
    .fetch_optional(&state.pool)
    .await?;
    Ok(granted.unwrap_or(true))
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
}
