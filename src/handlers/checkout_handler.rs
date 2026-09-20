use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

pub async fn list_payment_providers(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows: Vec<(Uuid, Option<Uuid>, String, String, bool, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, tenant_id, provider_type, COALESCE(api_key,''), is_active, created_at FROM payment_providers WHERE tenant_id = $1 ORDER BY created_at"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn upsert_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let provider = payload["provider_type"]
        .as_str()
        .ok_or_else(|| AppError::Validation("provider_type required".into()))?;
    let api_key = payload["api_key"].as_str().unwrap_or("");
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("INSERT INTO payment_providers (id, tenant_id, provider_type, api_key, is_active) VALUES ($1, $2, $3, $4, true)")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(provider).bind(api_key).execute(&state.pool).await?;
    Ok((StatusCode::OK, Json(json!({"message": "Provider saved"}))))
}
pub async fn delete_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider_type): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("DELETE FROM payment_providers WHERE provider_type = $1 AND tenant_id = $2")
        .bind(&provider_type)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Provider deleted"})))
}
/// POST /api/v1/checkout/create
///
/// Fails LOUDLY. There is no payment provider integration in FunnelSwift yet
/// (which provider is canonical — Stripe / Mintbird / Groovesell — is a product
/// decision). This used to return a fake `cs_test_placeholder` session id, so a
/// broken checkout looked successful. It now refuses with an explicit reason and
/// never creates a session or a charge.
pub async fn create_checkout_session(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();

    let configured: Option<(String,)> = sqlx::query_as(
        "SELECT provider_type FROM payment_providers WHERE tenant_id = $1 AND is_active = true ORDER BY created_at LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    match configured {
        None => {
            tracing::error!(
                "checkout/create REFUSED: no active payment provider for tenant {tenant_id}"
            );
            Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "payment_provider_not_configured",
                    "message": "No payment provider is configured for this account, so checkout cannot work. An admin must add the provider's keys in the admin panel (Provider Keys / Payment providers). No session was created and no charge can occur.",
                    "configured": false
                })),
            ))
        }
        Some((provider_type,)) => {
            tracing::error!(
                "checkout/create REFUSED: {provider_type} configured but live checkout is not implemented"
            );
            Ok((
                StatusCode::NOT_IMPLEMENTED,
                Json(json!({
                    "error": "checkout_not_implemented",
                    "message": format!("A {provider_type} provider is configured, but live checkout session creation is not implemented in FunnelSwift yet. No session was created and no charge can occur."),
                    "configured": true,
                    "provider_type": provider_type
                })),
            ))
        }
    }
}

/// GET /api/v1/checkout/session/:id
///
/// Public, id-scoped lookup used by the thank-you pages after the payment
/// provider redirects the buyer back. Exposes only presentation-safe fields —
/// never tenant_id, user_id, provider_session_id or metadata.
pub async fn get_checkout_session_public(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let Ok(id) = Uuid::parse_str(&session_id) else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({
                "found": false,
                "error": "checkout_session_not_found",
                "note": "That checkout id is not a valid session id."
            })),
        ));
    };

    let row = sqlx::query(
        r#"SELECT cs.id, cs.purchasable_type, cs.amount::text AS amount,
                  cs.currency, cs.status, cs.created_at,
                  p.name AS plan_name
           FROM checkout_sessions cs
           LEFT JOIN plans p
                  ON cs.purchasable_type = 'plan' AND p.id = cs.purchasable_id
           WHERE cs.id = $1"#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        tracing::warn!("checkout session lookup miss: {id}");
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({
                "found": false,
                "error": "checkout_session_not_found",
                "note": "No checkout session with that id. If you just paid, your provider receipt is authoritative — contact support if your plan is not active."
            })),
        ));
    };

    Ok((
        StatusCode::OK,
        Json(json!({
            "found": true,
            "id": row.try_get::<Uuid,_>("id").map(|u| u.to_string()).unwrap_or_default(),
            "status": row.try_get::<String,_>("status").unwrap_or_default(),
            "plan_name": row.try_get::<Option<String>,_>("plan_name").unwrap_or(None),
            "purchasable_type": row.try_get::<String,_>("purchasable_type").unwrap_or_default(),
            "amount": row.try_get::<String,_>("amount").unwrap_or_else(|_| "0".to_string()),
            "currency": row.try_get::<String,_>("currency").unwrap_or_else(|_| "USD".to_string()),
            "login_url": "/login.html",
            "created_at": row
                .try_get::<chrono::DateTime<chrono::Utc>,_>("created_at")
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
        })),
    ))
}

pub async fn list_checkout_sessions(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows = sqlx::query(
        r#"SELECT id, provider_type, purchasable_type, purchasable_id::text,
                  amount::text, currency, status, provider_session_id, created_at
           FROM checkout_sessions
           WHERE tenant_id = $1
           ORDER BY created_at DESC
           LIMIT 50"#,
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let sessions: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid,_>("id").map(|u| u.to_string()).unwrap_or_default(),
                "provider_type": r.try_get::<String,_>("provider_type").unwrap_or_default(),
                "purchasable_type": r.try_get::<String,_>("purchasable_type").unwrap_or_default(),
                "purchasable_id": r.try_get::<Option<String>,_>("purchasable_id").unwrap_or(None),
                "amount": r.try_get::<String,_>("amount").unwrap_or_else(|_| "0".to_string()),
                "currency": r.try_get::<String,_>("currency").unwrap_or_else(|_| "USD".to_string()),
                "status": r.try_get::<String,_>("status").unwrap_or_default(),
                "created_at": r
                    .try_get::<chrono::DateTime<chrono::Utc>,_>("created_at")
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(json!({ "sessions": sessions })))
}
pub async fn stripe_webhook(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::OK, Json(json!({"received": true}))))
}
pub async fn paypal_webhook(
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::OK, Json(json!({"received": true}))))
}
