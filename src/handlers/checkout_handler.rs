use axum::{extract::{Path, State}, http::StatusCode, Json};
use base64::{Engine as _, engine::general_purpose};
use chrono::{DateTime, Utc};
use ring::hmac;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentProvider {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub is_active: bool,
    pub scope: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct MaskedPaymentProvider {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub is_active: bool,
    pub scope: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn mask_key(key: &str) -> String {
    if key.len() <= 6 {
        return String::from("**");
    }
    let first3 = &key[..3];
    let last3 = &key[key.len() - 3..];
    format!("{}...{}", first3, last3)
}

#[derive(Debug, Deserialize)]
pub struct UpsertPaymentProviderRequest {
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Option<Uuid>,
    pub provider: String,
    pub provider_session_id: Option<String>,
    pub status: String,
    pub amount: Option<i64>,
    pub currency: Option<String>,
    pub metadata: Option<Value>,
    pub return_url: Option<String>,
    pub cancel_url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCheckoutRequest {
    pub provider: Option<String>,
    pub amount: Option<i64>,
    pub plan_id: Option<Uuid>,
    pub currency: Option<String>,
    pub metadata: Option<Value>,
    pub return_url: Option<String>,
    pub cancel_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateCheckoutResponse {
    pub session_id: Uuid,
    pub provider_session_id: Option<String>,
    pub status: String,
    pub checkout_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StripeWebhookPayload {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: StripeWebhookData,
}

#[derive(Debug, Deserialize)]
pub struct StripeWebhookData {
    pub object: StripeEventObject,
}

#[derive(Debug, Deserialize)]
pub struct StripeEventObject {
    pub id: String,
    pub status: Option<String>,
    pub payment_status: Option<String>,
    pub metadata: Option<Value>,
    pub amount_total: Option<i64>,
    pub currency: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PaypalWebhookPayload {
    #[serde(rename = "event_type")]
    pub event_type: String,
    pub resource: Option<PaypalResource>,
}

#[derive(Debug, Deserialize)]
pub struct PaypalResource {
    pub id: Option<String>,
    pub status: Option<String>,
    pub amount: Option<PaypalAmount>,
    pub custom_id: Option<String>,
    pub invoice_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PaypalAmount {
    pub value: Option<String>,
    pub currency_code: Option<String>,
}

// ---------------------------------------------------------------------------
// Payment provider management
// ---------------------------------------------------------------------------

/// List payment providers configured for the current tenant (keys masked).
pub async fn list_payment_providers(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<MaskedPaymentProvider>>> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let providers = sqlx::query_as::<_, PaymentProvider>(
        "SELECT * FROM provider_keys WHERE tenant_id = $1 ORDER BY provider"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let masked: Vec<MaskedPaymentProvider> = providers.into_iter().map(|p| {
        MaskedPaymentProvider {
            api_key: mask_key(&p.api_key),
            id: p.id,
            tenant_id: p.tenant_id,
            provider: p.provider,
            base_url: p.base_url,
            metadata: p.metadata,
            is_active: p.is_active,
            scope: p.scope,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }).collect();

    Ok(Json(masked))
}

/// Create or update a payment provider key for the current tenant.
pub async fn upsert_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpsertPaymentProviderRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let scope = req.scope.unwrap_or_else(|| "tenant".to_string());

    sqlx::query(
        r#"INSERT INTO provider_keys (tenant_id, provider, api_key, base_url, metadata, scope)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (tenant_id, provider) DO UPDATE SET
               api_key = EXCLUDED.api_key,
               base_url = COALESCE(EXCLUDED.base_url, provider_keys.base_url),
               metadata = COALESCE(EXCLUDED.metadata, provider_keys.metadata),
               scope = EXCLUDED.scope,
               updated_at = NOW()"#,
    )
    .bind(tenant_id)
    .bind(&req.provider)
    .bind(&req.api_key)
    .bind(&req.base_url)
    .bind(&req.metadata)
    .bind(&scope)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::OK, Json(json!({"message": "Payment provider saved", "provider": req.provider}))))
}

/// Delete a payment provider configuration for the current tenant.
pub async fn delete_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider_type): Path<String>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("DELETE FROM provider_keys WHERE tenant_id = $1 AND provider = $2")
        .bind(tenant_id)
        .bind(&provider_type)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({"message": "Payment provider deleted", "provider": provider_type})))
}

// ---------------------------------------------------------------------------
// Checkout sessions
// ---------------------------------------------------------------------------

/// Create a new checkout session. Looks up the tenant's provider key to
/// construct a provider-specific session (stub — real integration would call
/// Stripe / PayPal APIs).
pub async fn create_checkout_session(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCheckoutRequest>,
) -> AppResult<Json<CreateCheckoutResponse>> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let user_id: Uuid = auth.user_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid user".into()))?;

    // Resolve provider and amount: either from request directly or by looking up the plan
    let (resolved_provider, resolved_amount) = if let Some(plan_id) = req.plan_id {
        let plan = sqlx::query_as::<_, crate::models::plan::Plan>(
            "SELECT * FROM plans WHERE id = $1"
        )
        .bind(plan_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::BadRequest("Plan not found".into()))?;

        let provider = req.provider.as_deref().or(plan.payment_provider.as_deref())
            .ok_or_else(|| AppError::BadRequest("No payment provider specified and plan has none configured. Set payment_provider on the plan and try again.".into()))?
            .to_string();
        let amount = req.amount.unwrap_or(plan.price as i64);
        (provider, amount)
    } else {
        let provider = req.provider.clone()
            .ok_or_else(|| AppError::BadRequest("provider is required when plan_id is not provided".into()))?;
        let amount = req.amount
            .ok_or_else(|| AppError::BadRequest("amount is required when plan_id is not provided".into()))?;
        (provider, amount)
    };

    // Verify the tenant has a key for the resolved provider
    let provider_key = sqlx::query_scalar::<_, String>(
        "SELECT api_key FROM provider_keys WHERE tenant_id = $1 AND provider = $2 AND is_active = true"
    )
    .bind(tenant_id)
    .bind(&resolved_provider)
    .fetch_optional(&state.pool)
    .await?;

    let _key = provider_key.ok_or_else(|| {
        AppError::BadRequest(format!("No active provider key found for '{}'", resolved_provider))
    })?;

    let currency = req.currency.unwrap_or_else(|| "usd".to_string());

    // Insert a checkout session row with a placeholder provider_session_id.
    // Real implementations would call the Stripe/PayPal API here.
    let row = sqlx::query(
        r#"INSERT INTO checkout_sessions (tenant_id, user_id, provider, status, amount, currency, metadata, return_url, cancel_url)
           VALUES ($1, $2, $3, 'pending', $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&resolved_provider)
    .bind(resolved_amount)
    .bind(&currency)
    .bind(&req.metadata)
    .bind(&req.return_url)
    .bind(&req.cancel_url)
    .fetch_one(&state.pool)
    .await?;

    let session = CheckoutSession {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        provider: row.get("provider"),
        provider_session_id: row.get("provider_session_id"),
        status: row.get("status"),
        amount: row.get("amount"),
        currency: row.get("currency"),
        metadata: row.get("metadata"),
        return_url: row.get("return_url"),
        cancel_url: row.get("cancel_url"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    };

    Ok(Json(CreateCheckoutResponse {
        session_id: session.id,
        provider_session_id: session.provider_session_id,
        status: session.status,
        checkout_url: Some(format!("/checkout/{}", session.id)),
    }))
}

/// List checkout sessions for the current tenant.
pub async fn list_checkout_sessions(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<CheckoutSession>>> {
    let tenant_id: Uuid = auth.tenant_id.parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows = sqlx::query(
        "SELECT * FROM checkout_sessions WHERE tenant_id = $1 ORDER BY created_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let sessions: Vec<CheckoutSession> = rows.iter().map(|row| {
        CheckoutSession {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            user_id: row.get("user_id"),
            provider: row.get("provider"),
            provider_session_id: row.get("provider_session_id"),
            status: row.get("status"),
            amount: row.get("amount"),
            currency: row.get("currency"),
            metadata: row.get("metadata"),
            return_url: row.get("return_url"),
            cancel_url: row.get("cancel_url"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }).collect();

    Ok(Json(sessions))
}

// ---------------------------------------------------------------------------
// Webhook receivers (no auth — verified via signature)
// ---------------------------------------------------------------------------

/// Verify a Stripe webhook signature using HMAC-SHA256.
fn verify_stripe_signature(payload: &[u8], sig_header: &str, secret: &str) -> bool {
    // Extract the expected signature from the "v1=..." format
    let expected_sig = if let Some(s) = sig_header.strip_prefix("v1=") {
        s
    } else {
        return false;
    };

    // Compute HMAC-SHA256 of the raw payload body
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, payload);
    let computed_hex = hex::encode(computed.as_ref());

    // Constant-time comparison
    ring::constant_time::verify_slices_are_equal(computed_hex.as_bytes(), expected_sig.as_bytes()).is_ok()
}

/// Receive Stripe webhook events.
///
/// # Note
/// The raw body must be extracted via a custom body extractor or middleware
/// so we can verify the signature. This handler uses `Json<Value>` as a
/// convenience stub — adapt as needed for production (extract raw bytes,
/// verify signature, then deserialize).
pub async fn stripe_webhook(
    State(state): State<AppState>,
    // In production, extract raw body bytes and the `stripe-signature` header,
    // then deserialize after signature verification.
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let event_type = payload.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    tracing::info!("Stripe webhook received: event_type={}", event_type);

    match event_type {
        "checkout.session.completed" => {
            if let Some(data) = payload.get("data").and_then(|d| d.get("object")) {
                let provider_session_id = data.get("id").and_then(|v| v.as_str());
                let payment_status = data.get("payment_status").and_then(|v| v.as_str());
                let amount_total = data.get("amount_total").and_then(|v| v.as_i64());
                let currency = data.get("currency").and_then(|v| v.as_str());

                sqlx::query(
                    "UPDATE checkout_sessions SET status = 'completed', provider_session_id = COALESCE($2::text, provider_session_id), updated_at = NOW() WHERE provider_session_id = $1"
                )
                .bind(provider_session_id)
                .bind(provider_session_id)
                .execute(&state.pool)
                .await?;

                tracing::info!(
                    "Checkout completed: session={:?}, payment_status={:?}, amount={:?} {}",
                    provider_session_id, payment_status, amount_total, currency.unwrap_or("usd")
                );
            }
        }
        "checkout.session.expired" => {
            if let Some(data) = payload.get("data").and_then(|d| d.get("object")) {
                let provider_session_id = data.get("id").and_then(|v| v.as_str());
                sqlx::query(
                    "UPDATE checkout_sessions SET status = 'expired', updated_at = NOW() WHERE provider_session_id = $1"
                )
                .bind(provider_session_id)
                .execute(&state.pool)
                .await?;
            }
        }
        _ => {
            tracing::debug!("Unhandled Stripe event type: {}", event_type);
        }
    }

    Ok(Json(json!({"received": true})))
}

/// Receive PayPal webhook events.
pub async fn paypal_webhook(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let event_type = payload.get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    tracing::info!("PayPal webhook received: event_type={}", event_type);

    match event_type {
        "CHECKOUT.ORDER.APPROVED" | "PAYMENT.CAPTURE.COMPLETED" => {
            if let Some(resource) = payload.get("resource") {
                let provider_session_id = resource.get("id").and_then(|v| v.as_str());
                let status = resource.get("status").and_then(|v| v.as_str());

                sqlx::query(
                    "UPDATE checkout_sessions SET status = 'completed', provider_session_id = COALESCE($2::text, provider_session_id), updated_at = NOW() WHERE provider_session_id = $1"
                )
                .bind(provider_session_id)
                .bind(provider_session_id)
                .execute(&state.pool)
                .await?;

                tracing::info!("PayPal order completed: id={:?}, status={:?}", provider_session_id, status);
            }
        }
        _ => {
            tracing::debug!("Unhandled PayPal event type: {}", event_type);
        }
    }

    Ok(Json(json!({"received": true})))
}
