//! Feature limits enforcement for FunnelSwift.
//! Reads plan limits from the plans table (max_cards, max_leads, etc.).
//! Falls back to feature_limits table for any custom limits defined there.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use uuid::Uuid;

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
    .await
    .unwrap_or(None)
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

    // Fall back to plans table columns
    let plan_col = match feature_key {
        "max_cards" | "max_kinetic_cards" => "max_cards",
        "max_leads" => "max_leads",
        "max_tags" => "max_tags",
        "max_forms" => "max_forms",
        "max_custom_domains" => "max_domains",
        "max_team_members" | "team_members" => "max_team_members",
        "max_webhooks" => "max_webhooks",
        "max_api_keys" => "max_api_keys",
        "max_portfolios" => "max_portfolios",
        "max_affiliates" => "max_affiliates",
        "max_tag_groups" => "max_tag_groups",
        "max_routing_targets" => "max_routing_targets",
        "max_integrations" => "max_integrations",
        _ => return Ok(()), // unknown feature — allow
    };

    let limit_val: Option<i32> = sqlx::query_scalar(&format!(
        "SELECT p.{} FROM plans p
         JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
         WHERE tps.tenant_id = $1 AND tps.status = 'active'
         ORDER BY tps.start_date DESC LIMIT 1",
        plan_col
    ))
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None)
    .flatten();

    match limit_val {
        None => Ok(()), // No plan assigned or no limit set — allow
        Some(limit) => {
            if limit == -1 {
                return Ok(());
            } // unlimited
            if limit == 0 {
                return Err(AppError::UpgradeRequired(format!(
                    "{} is not available on your current plan. Upgrade to access this feature.",
                    label
                )));
            }
            let usage = get_usage_count(state, tenant_id, feature_key).await;
            if usage >= limit as i64 {
                return Err(AppError::UpgradeRequired(format!(
                    "{} limit reached ({}/{}). Upgrade to increase your limit.",
                    label, usage, limit
                )));
            }
            Ok(())
        }
    }
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
        "max_forms" => sqlx::query_scalar("SELECT COUNT(*) FROM forms WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0),
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
            sqlx::query_scalar("SELECT COUNT(*) FROM portfolios WHERE tenant_id = $1")
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
            sqlx::query_scalar("SELECT COUNT(*) FROM routing_targets WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
        "max_integrations" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM integration_targets WHERE tenant_id = $1")
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
        "max_custom_domains" | "max_domains" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM custom_domains WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(0)
        }
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
