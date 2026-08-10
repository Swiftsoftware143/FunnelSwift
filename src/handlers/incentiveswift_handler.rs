use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// Returns IncentiveSwift connection config for the FunnelSwift mobile app.
/// Looks up the user's IncentiveSwift API key from their integration targets
/// (set via the FunnelSwift Integration Center in the web admin).
pub async fn get_incentiveswift_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| crate::error::AppError::BadRequest("Invalid tenant".into()))?;

    let base_url = std::env::var("IS_BASE_URL")
        .unwrap_or_else(|_| "https://incentiveswift.com".to_string());

    // Look up the tenant's IncentiveSwift integration target
    let row = sqlx::query(
        "SELECT api_key, webhook_url, is_active FROM target_software WHERE tenant_id = $1 AND LOWER(name) LIKE '%incentiveswift%' AND is_active = true LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let (api_key, campaigns_url, connected) = if let Some(ref r) = row {
        let key: Option<String> = r.try_get("api_key").unwrap_or(None);
        let is_active: bool = r.try_get("is_active").unwrap_or(false);
        let has_key = key.as_ref().map(|k| !k.is_empty()).unwrap_or(false);
        let campaigns = format!("{}/api/v1/campaigns", base_url);
        (key.unwrap_or_default(), campaigns, has_key && is_active)
    } else {
        (String::new(), format!("{}/api/v1/campaigns", base_url), false)
    };

    Ok(Json(json!({
        "connected": connected,
        "api_key": api_key,
        "base_url": base_url,
        "campaigns_url": campaigns_url,
        "enabled": true
    })))
}
