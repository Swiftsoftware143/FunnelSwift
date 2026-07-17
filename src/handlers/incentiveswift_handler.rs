//! IncentiveSwift handler — provides the mobile app with IncentiveSwift API key
//! so it can call IncentiveSwift's API directly for clean campaign display.

use axum::{extract::State, Json};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// GET /api/v1/incentiveswift/config — return the user's IncentiveSwift API key
/// and base URL so the mobile app can call IncentiveSwift directly.
pub async fn get_incentiveswift_config(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let tenant_id: Uuid = user.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let row = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT api_key, base_url
         FROM provider_keys
         WHERE tenant_id = $1 AND provider = 'incentiveswift' AND is_active = true
         LIMIT 1"
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    match row {
        Some((api_key, base_url)) => {
            let url = base_url
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| "https://app.incentiveswift.com".to_string());
            Ok(Json(json!({
                "connected": true,
                "api_key": api_key,
                "base_url": url,
                "campaigns_url": format!("{}/api/v1/campaigns", url.trim_end_matches('/')),
                "warning": "Keep this key secure — it grants access to your IncentiveSwift account"
            })))
        }
        None => Ok(Json(json!({
            "connected": false,
            "api_key": null,
            "base_url": null,
            "campaigns_url": null
        }))),
    }
}
