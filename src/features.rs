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
            "SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1 AND is_template = false",
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
