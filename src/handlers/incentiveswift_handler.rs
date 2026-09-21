use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::security::provider_key_crypto;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// Returns IncentiveSwift connection config for the FunnelSwift mobile app.
/// Looks up the user's IncentiveSwift API key from their integration targets
/// (set via the FunnelSwift Integration Center in the web admin).
///
/// CREDENTIAL CONTRACT (kanban t_63840ff2)
/// --------------------------------------
/// `target_software.api_key` is CIPHERTEXT at rest, so it is decrypted here — a ciphertext can
/// never be a usable bearer. This endpoint is a deliberate CONFIG HAND-OFF, not a read-back: the
/// caller is the tenant's own authenticated device and the key being handed over is that tenant's
/// own credential, which the app then uses as `Authorization: Bearer <key>` when it calls
/// IncentiveSwift directly (FunnelSwift-Mobile `getIncentiveSwiftConfig` -> `getCampaigns`,
/// CampaignsPicker). Masking it would silently break campaign loading, so the plaintext stays on
/// THIS response only, while `api_key_masked` carries the same shape every other credential
/// surface uses. No other endpoint returns the key, and the ciphertext is never returned.
pub async fn get_incentiveswift_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| crate::error::AppError::BadRequest("Invalid tenant".into()))?;

    let base_url =
        std::env::var("IS_BASE_URL").unwrap_or_else(|_| "https://incentiveswift.com".to_string());

    // Look up the tenant's IncentiveSwift integration target
    let row = sqlx::query(
        "SELECT api_key, webhook_url, is_active FROM target_software WHERE tenant_id = $1 AND LOWER(name) LIKE '%incentiveswift%' AND is_active = true LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let stored: Option<String> = row
        .as_ref()
        .and_then(|r| r.try_get("api_key").unwrap_or(None));
    let is_active = row
        .as_ref()
        .map(|r| r.try_get::<bool, _>("is_active").unwrap_or(false))
        .unwrap_or(false);

    // Decrypt for USE (never return the stored bytes).
    let key = provider_key_crypto::decrypt_optional(&state.pool, stored.as_deref()).await?;
    let has_key = key.as_ref().map(|k| !k.is_empty()).unwrap_or(false);
    let campaigns_url = format!("{}/api/v1/campaigns", base_url);

    Ok(Json(json!({
        "connected": has_key && is_active,
        "api_key": key.clone().unwrap_or_default(),
        "api_key_masked": key
            .as_deref()
            .map(provider_key_crypto::mask)
            .unwrap_or_default(),
        "base_url": base_url,
        "campaigns_url": campaigns_url,
        "enabled": true
    })))
}
