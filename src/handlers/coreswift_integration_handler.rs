//! Canonical CoreSwift spoke endpoints (fleet standard 2026-09-20 §Canonical spoke endpoints).
//!
//!   GET  /api/v1/integrations/coreswift/status  -> {"connected": bool, "base_url": "..."}
//!   GET  /api/v1/integrations/coreswift/lists   -> proxy hub GET /api/external/lists
//!   POST /api/v1/integrations/coreswift/push    -> proxy hub POST /api/external/contacts
//!
//! Every one of these delegates to `crate::coreswift` — the single CoreSwift implementation in
//! this repo. There is no second CoreSwift client.

use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::coreswift::{self, LeadPayload};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

fn tenant_of(auth: &AuthUser) -> AppResult<Uuid> {
    Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))
}

/// GET /api/v1/integrations/coreswift/status
pub async fn coreswift_status(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth)?;
    let conn = coreswift::resolve_conn(&state.pool, tenant_id)
        .await
        .map_err(AppError::Internal)?;

    let (connected, base_url, key_preview) = match conn {
        Some(c) => (true, c.base_url.clone(), Some(c.key_preview())),
        None => (
            false,
            coreswift::resolved_base_url(&state.pool, None).await,
            None,
        ),
    };

    Ok(Json(json!({
        "provider": "coreswift",
        "name": "CoreSwift CRM",
        "connected": connected,
        "base_url": base_url,
        "key_preview": key_preview,
        "direction": "inbound",
        "description": "Push leads into CoreSwift CRM",
    })))
}

/// GET /api/v1/integrations/coreswift/lists — proxy for the hub's list picker.
pub async fn coreswift_lists(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth)?;
    let conn = coreswift::resolve_conn(&state.pool, tenant_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::BadRequest("CoreSwift is not connected".into()))?;

    let lists = coreswift::hub_lists(&conn)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(json!({
        "provider": "coreswift",
        "base_url": conn.base_url,
        "lists": lists,
    })))
}

/// POST /api/v1/integrations/coreswift/push — manual fallback for the automatic capture push.
pub async fn coreswift_push(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth)?;
    let has_identity = ["email", "phone", "name"]
        .iter()
        .any(|k| payload.get(k).and_then(|v| v.as_str()).is_some());
    if !has_identity {
        return Err(AppError::BadRequest(
            "email, phone or name is required".into(),
        ));
    }

    if coreswift::resolve_conn(&state.pool, tenant_id)
        .await
        .map_err(AppError::Internal)?
        .is_none()
    {
        return Err(AppError::BadRequest(
            "CoreSwift is not connected — store a csk_ key first".into(),
        ));
    }

    let mut fields = serde_json::Map::new();
    if let Some(list_id) = payload.get("list_id") {
        fields.insert("list_id".into(), list_id.clone());
    }
    let tags: Vec<String> = payload
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let lead = LeadPayload {
        email: payload
            .get("email")
            .and_then(|v| v.as_str())
            .map(String::from),
        phone: payload
            .get("phone")
            .and_then(|v| v.as_str())
            .map(String::from),
        name: payload
            .get("name")
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("first_name").and_then(|v| v.as_str()))
            .map(String::from),
        company: payload
            .get("company")
            .and_then(|v| v.as_str())
            .map(String::from),
        tags,
        source: Some("manual".to_string()),
        fields,
    };

    let pushed = coreswift::push_lead_to_coreswift(&state.pool, tenant_id, lead)
        .await
        .map_err(AppError::Internal)?;

    Ok(Json(json!({
        "provider": "coreswift",
        "status": if pushed { "pushed" } else { "skipped" },
        "message": if pushed {
            "Lead pushed to CoreSwift CRM".to_string()
        } else {
            "Nothing to push (no email/phone/name)".to_string()
        },
    })))
}

/// POST /api/v1/provider-keys/:provider/test — live probe of a stored BYOK credential.
/// For `coreswift` this is a REAL hub call (lists). For the others it reports whether a key is
/// stored without claiming a validation the app cannot perform.
pub async fn test_provider_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth)?;

    if provider == "coreswift" {
        let Some(conn) = coreswift::resolve_conn(&state.pool, tenant_id)
            .await
            .map_err(AppError::Internal)?
        else {
            return Ok(Json(json!({
                "provider": provider,
                "valid": false,
                "detail": "No CoreSwift key stored for this tenant",
            })));
        };
        return match coreswift::hub_lists(&conn).await {
            Ok(lists) => {
                let count = lists
                    .get("lists")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                Ok(Json(json!({
                    "provider": provider,
                    "valid": true,
                    "base_url": conn.base_url,
                    "detail": format!("CoreSwift hub reachable — {count} list(s) visible to this key"),
                })))
            }
            Err(e) => Ok(Json(json!({
                "provider": provider,
                "valid": false,
                "base_url": conn.base_url,
                "detail": e,
            }))),
        };
    }

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT COALESCE(api_key, '') FROM provider_keys \
         WHERE tenant_id = $1 AND provider = $2 AND is_active = true LIMIT 1",
    )
    .bind(tenant_id)
    .bind(&provider)
    .fetch_optional(&state.pool)
    .await?;
    // The stored value is ciphertext at rest: "has a key" means the DECRYPTED value is
    // non-empty, which also proves the master key can read what the app wrote.
    let has_key = match stored.as_deref() {
        Some(k) if !k.trim().is_empty() => {
            !crate::security::provider_key_crypto::decrypt_from_storage(&state.pool, k.trim())
                .await?
                .trim()
                .is_empty()
        }
        _ => false,
    };
    Ok(Json(json!({
        "provider": provider,
        "valid": has_key,
        "detail": if has_key {
            "Key stored for this tenant (no provider-side probe available in FunnelSwift)"
        } else {
            "No key stored for this tenant"
        },
    })))
}
