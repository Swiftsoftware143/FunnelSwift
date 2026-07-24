//! Campaigns handler — proxies to IncentiveSwift using the user's linked API key.
//! Only used by the mobile app. Requires the user to connect their IncentiveSwift
//! account via a provider key (Settings → API Keys in the web app).

use axum::{extract::State, Json};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// GET /api/v1/campaigns — fetch campaigns from IncentiveSwift using the
/// user's linked API key (stored in provider_keys as provider='incentiveswift').
/// Returns empty array if no key is linked.
pub async fn list_campaigns(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let tenant_id: Uuid = user.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Look up the user's IncentiveSwift API key
    let row = sqlx::query_as::<_, (String,String)>(
        "SELECT api_key, COALESCE(base_url, 'http://localhost:8083') as base_url
         FROM provider_keys
         WHERE tenant_id = $1 AND provider = 'incentiveswift' AND is_active = true
         LIMIT 1"
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let (api_key, base_url) = match row {
        Some(r) => r,
        None => {
            // No IncentiveSwift API key linked — return empty
            return Ok(Json(json!({ "campaigns": [] })));
        }
    };

    let url = format!("{}/api/v1/campaigns", base_url.trim_end_matches('/'));

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(AppError::Reqwest)?;

    if !resp.status().is_success() {
        return Ok(Json(json!({ "campaigns": [] })));
    }

    let campaigns: Value = resp
        .json()
        .await
        .unwrap_or(json!({ "campaigns": [] }));

    Ok(Json(campaigns))
}
