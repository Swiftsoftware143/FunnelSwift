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
use sqlx::FromRow;
use uuid::Uuid;

/// Numeric limit, read from `feature_limits` (custom override) first, then the
/// active plan's concrete `plans` column. Returns `Ok(None)` when the tenant has
/// no active plan, the column is NULL, or the key is not a known plan column
/// (feature_limits-only keys such as max_webhooks / max_api_keys / max_portfolios
/// / max_affiliates / max_tag_groups / max_routing_targets / max_integrations are
/// resolved solely by the feature_limits lookup in `enforce_feature_limit`).
///
/// Each arm uses a fixed, allowlisted column literal — no dynamic SQL.
async fn plan_limit(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<Option<i32>> {
    let limit: Option<i32> = match feature_key {
        "max_cards" | "max_kinetic_cards" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_cards FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_leads" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_leads FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_tags" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_tags FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_forms" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_forms FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_custom_domains" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_custom_domains FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_team_members" | "team_members" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_team_members FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_qr_codes" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_qr_codes FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_action_buttons" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_action_buttons FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        "max_ocr_scans" => {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT p.max_ocr_scans FROM plans p JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id WHERE tps.tenant_id = $1 AND tps.status = 'active' ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
        }
        // feature_limits-only keys and unknown keys: no plan column to read.
        _ => None,
    };

    Ok(limit)
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
    // First check the feature_limits table (custom overrides)
    let fl: Option<i32> = sqlx::query_scalar(
        "SELECT fl.limit_value FROM feature_limits fl
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = fl.plan_id
         WHERE tps.tenant_id = $1 AND tps.status = 'active' AND fl.feature_key = $2
         ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(feature_key)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    if let Some(val) = fl {
        if val == -1 {
            return Ok(());
        } // unlimited
        if val == 0 {
            return Err(AppError::UpgradeRequired(format!(
                "{} is not available on your current plan. Upgrade to access this feature.",
                label
            )));
        }
        // Check usage against limit
        let usage = get_usage_count(state, tenant_id, feature_key).await;
        if usage >= val as i64 {
            return Err(AppError::UpgradeRequired(format!(
                "{} limit reached ({}/{}). Upgrade to increase your limit.",
                label, usage, val
            )));
        }
        return Ok(());
    }

    // Fall back to plans table columns (fixed, allowlisted — no dynamic SQL)
    let limit_val = plan_limit(state, tenant_id, feature_key).await?;
    match limit_val {
        None => Ok(()), // No plan assigned or no limit set — allow
        Some(limit) => {
            let usage = get_usage_count(state, tenant_id, feature_key).await;
            check_numeric_limit(limit, usage, label)
        }
    }
}

/// Boolean plan flags for the active tenant's plan.
#[derive(FromRow)]
struct PlanFlags {
    features: Option<serde_json::Value>,
    has_webhooks: bool,
    has_api: bool,
    has_dual_routing: bool,
    has_mini_funnels: bool,
    has_card_gating: bool,
    has_remove_branding: bool,
    has_white_label: bool,
    has_multi_tenant: bool,
    has_analytics: bool,
    has_import_export: bool,
}

/// Map a `has_*` column name to the corresponding `features` jsonb key.
/// Returns `None` for unknown flags (no jsonb override).
fn flag_jsonb_key(feature_key: &str) -> Option<&'static str> {
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
        _ => None,
    }
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
    let flags: Option<PlanFlags> = sqlx::query_as(
        "SELECT p.features, p.has_webhooks, p.has_api, p.has_dual_routing, p.has_mini_funnels,
                p.has_card_gating, p.has_remove_branding, p.has_white_label, p.has_multi_tenant,
                p.has_analytics, p.has_import_export
         FROM plans p
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
         WHERE tps.tenant_id = $1 AND tps.status = 'active'
         ORDER BY tps.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some(flags) = flags else {
        return Ok(()); // no active plan — allow (consistent with numeric gating)
    };

    // jsonb override when the key is present (single source of truth), else the column.
    let enabled = flag_jsonb_key(feature_key)
        .and_then(|k| {
            flags
                .features
                .as_ref()
                .and_then(|f| f.get(k))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(match feature_key {
            "has_webhooks" => flags.has_webhooks,
            "has_api" => flags.has_api,
            "has_dual_routing" => flags.has_dual_routing,
            "has_mini_funnels" => flags.has_mini_funnels,
            "has_card_gating" => flags.has_card_gating,
            "has_remove_branding" => flags.has_remove_branding,
            "has_white_label" => flags.has_white_label,
            "has_multi_tenant" => flags.has_multi_tenant,
            "has_analytics" => flags.has_analytics,
            "has_import_export" => flags.has_import_export,
            _ => false,
        });

    if enabled {
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
    let limit = plan_limit(state, tenant_id, "max_action_buttons").await?;
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
        "max_api_keys" => sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0),
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
        "max_routing_targets" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM target_software WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "max_integrations" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM target_software WHERE tenant_id = $1")
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
