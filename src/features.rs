//! Feature limits enforcement for FunnelSwift.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct FeatureLimitResult {
    pub allowed: bool,
    pub limit: i32,
    pub usage: i64,
    pub feature_key: String,
}

pub async fn check_feature_limit(
    state: &AppState,
    tenant_id: Uuid,
    feature_key: &str,
) -> AppResult<FeatureLimitResult> {
    let plan_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT plan_id FROM tenant_plan_subscriptions WHERE tenant_id = $1 AND status = 'active'"
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    let plan_id = match plan_id {
        Some(id) => id,
        None => return Ok(FeatureLimitResult { allowed: false, limit: 0, usage: 0, feature_key: feature_key.to_string() }),
    };

    let limit_value: Option<i32> = sqlx::query_scalar(
        "SELECT limit_value FROM feature_limits WHERE plan_id = $1 AND feature_key = $2"
    )
    .bind(plan_id)
    .bind(feature_key)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    let limit_value = match limit_value {
        Some(v) => v,
        None => return Ok(FeatureLimitResult { allowed: false, limit: 0, usage: 0, feature_key: feature_key.to_string() }),
    };

    if limit_value == -1 {
        return Ok(FeatureLimitResult { allowed: true, limit: -1, usage: 0, feature_key: feature_key.to_string() });
    }

    let usage: i64 = match feature_key {
        "max_leads" => sqlx::query_scalar("SELECT COUNT(*) FROM leads WHERE tenant_id = $1").bind(tenant_id).fetch_one(&state.pool).await?,
        "max_tags" => sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE tenant_id = $1").bind(tenant_id).fetch_one(&state.pool).await?,
        "max_affiliates" => sqlx::query_scalar("SELECT COUNT(*) FROM affiliates WHERE tenant_id = $1").bind(tenant_id).fetch_one(&state.pool).await?,
        "team_members" => sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE tenant_id = $1 AND is_active = true").bind(tenant_id).fetch_one(&state.pool).await?,
        _ => 0i64,
    };

    Ok(FeatureLimitResult {
        allowed: usage < limit_value as i64,
        limit: limit_value,
        usage,
        feature_key: feature_key.to_string(),
    })
}

pub async fn enforce_feature_limit(state: &AppState, tenant_id: Uuid, feature_key: &str, label: &str) -> AppResult<()> {
    let result = check_feature_limit(state, tenant_id, feature_key).await?;
    if !result.allowed {
        let msg = if result.limit == 0 {
            format!("{} is not available on your current plan. Upgrade to access this feature.", label)
        } else {
            format!("{} limit reached ({}/{})", label, result.usage, result.limit)
        };
        return Err(AppError::BadRequest(msg));
    }
    Ok(())
}
