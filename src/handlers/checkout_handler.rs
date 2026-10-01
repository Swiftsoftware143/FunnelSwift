use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

pub async fn list_payment_providers(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // created_at is TIMESTAMPTZ in this table: decoding it as NaiveDateTime failed the query, and
    // `unwrap_or_default()` turned that decode error into a silent EMPTY list — every configured
    // payment provider was invisible. It now decodes as DateTime<Utc>, and a real DB error is a 500
    // instead of a lie.
    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        String,
        String,
        bool,
        chrono::DateTime<chrono::Utc>,
        bool,
    )> = sqlx::query_as(
        "SELECT id, tenant_id, provider_type, COALESCE(api_key,''), is_active, created_at,
                (webhook_secret IS NOT NULL AND webhook_secret <> '') AS has_webhook_secret
           FROM payment_providers WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    // payment_providers.api_key is CIPHERTEXT at rest (src/security/provider_key_crypto.rs), so
    // the read-back is decrypted HERE and only a mask of the DECRYPTED value is returned. Before
    // kanban t_63840ff2 this endpoint put the raw secret key in the response body.
    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for (id, tenant_id, provider_type, stored, is_active, created_at, has_webhook_secret) in rows {
        let plain =
            crate::security::provider_key_crypto::decrypt_from_storage(&state.pool, stored.trim())
                .await?;
        out.push(json!({
            "id": id,
            "tenant_id": tenant_id,
            "provider_type": provider_type,
            "api_key_masked": crate::security::provider_key_crypto::mask(&plain),
            // Whether a webhook signing secret is stored — never the secret itself. The receiver
            // cannot verify a single delivery without one, so a provider row missing it must SAY so
            // here rather than look complete.
            "webhook_secret_set": has_webhook_secret,
            "is_active": is_active,
            "created_at": created_at,
        }));
    }
    Ok(Json(json!(out)))
}
pub async fn upsert_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let provider = payload["provider_type"]
        .as_str()
        .ok_or_else(|| AppError::Validation("provider_type required".into()))?;
    let api_key = payload["api_key"].as_str().unwrap_or("").trim().to_string();
    // Stripe signs each delivery with a webhook signing secret that is DIFFERENT from the API key.
    // Without it the receiver can verify nothing, so it is collected here and encrypted exactly like
    // the API key: the column only ever holds 'enc:v1:' ciphertext and a write fails closed (500)
    // without the master key rather than storing a secret in the clear.
    let webhook_secret = payload["webhook_secret"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let stored_api_key = if api_key.is_empty() {
        String::new()
    } else {
        crate::security::provider_key_crypto::encrypt_for_storage(&state.pool, &api_key).await?
    };
    let stored_secret = if webhook_secret.is_empty() {
        String::new()
    } else {
        crate::security::provider_key_crypto::encrypt_for_storage(&state.pool, &webhook_secret)
            .await?
    };
    // Upsert: re-saving with a blank field KEEPS the stored value, so an admin can correct one key
    // without having to re-paste the other. Before this a second save collided on the unique index
    // (tenant_id, provider_type) and returned a 500.
    sqlx::query(
        "INSERT INTO payment_providers (id, tenant_id, provider_type, api_key, webhook_secret, is_active)
         VALUES ($1, $2, $3, $4, $5, true)
         ON CONFLICT (tenant_id, provider_type) DO UPDATE SET
             api_key = COALESCE(NULLIF(EXCLUDED.api_key, ''), payment_providers.api_key),
             webhook_secret = COALESCE(NULLIF(EXCLUDED.webhook_secret, ''), payment_providers.webhook_secret),
             is_active = true",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(provider)
    .bind(&stored_api_key)
    .bind(&stored_secret)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::OK,
        Json(json!({
            "message": "Provider saved",
            "provider_type": provider,
            "api_key_masked": crate::security::provider_key_crypto::mask(&api_key),
            "webhook_secret_set": !webhook_secret.is_empty(),
        })),
    ))
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
    Json(payload): Json<Value>,
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
            // David, 2026-10-01: *"i will use stripe for now"*. This used to answer 501
            // "checkout_not_implemented" — i.e. a customer who had been sold an upgrade could not buy
            // it, so nothing downstream (the affiliate credit, the dated movement) could ever fire.
            if provider_type != "stripe" {
                return Ok((
                    StatusCode::NOT_IMPLEMENTED,
                    Json(json!({
                        "error": "checkout_not_implemented",
                        "message": format!("A {provider_type} provider is configured, but only Stripe checkout is implemented. No session was created and no charge can occur."),
                        "configured": true,
                        "provider_type": provider_type
                    })),
                ));
            }
            let plan_slug = payload["plan_slug"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            if plan_slug.is_empty() {
                return Err(AppError::Validation("plan_slug is required".into()));
            }
            let plan: Option<(Uuid, String, f64)> = sqlx::query_as(
                "SELECT id, name, COALESCE(price, 0)::float8 FROM plans WHERE slug = $1",
            )
            .bind(&plan_slug)
            .fetch_optional(&state.pool)
            .await?;
            let Some((plan_id, plan_name, price)) = plan else {
                return Err(AppError::NotFound("Unknown plan".into()));
            };
            if price <= 0.0 {
                return Err(AppError::Validation(
                    "That plan costs nothing — there is nothing to check out.".into(),
                ));
            }
            let key: Option<String> = sqlx::query_scalar(
                "SELECT api_key FROM payment_providers WHERE tenant_id = $1 AND provider_type = 'stripe' AND is_active = true LIMIT 1",
            )
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
            let key =
                crate::security::provider_key_crypto::decrypt_optional(&state.pool, key.as_deref())
                    .await
                    .unwrap_or(None)
                    .filter(|k| !k.trim().is_empty());
            let Some(secret_key) = key else {
                tracing::error!("checkout/create REFUSED: stripe provider row has no usable api_key for tenant {tenant_id}");
                return Ok((
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": "payment_provider_not_configured",
                        "message": "A Stripe provider exists but its secret key is missing or unreadable, so no session was created and no charge can occur. An admin must re-enter the key in the admin panel.",
                        "configured": false
                    })),
                ));
            };
            match create_stripe_session(
                tenant_id,
                &auth.user_id,
                plan_id,
                &plan_name,
                price,
                &secret_key,
            )
            .await
            {
                Ok((url, provider_session_id)) => {
                    sqlx::query(
                        "INSERT INTO checkout_sessions (id, tenant_id, user_id, provider_type, provider_session_id, purchasable_type, purchasable_id, amount, status, metadata)
                         VALUES ($1, $2, $3, 'stripe', $4, 'plan', $5, $6, 'pending', $7)",
                    )
                    .bind(Uuid::new_v4())
                    .bind(tenant_id)
                    .bind(Uuid::parse_str(&auth.user_id).ok())
                    .bind(&provider_session_id)
                    .bind(plan_id)
                    .bind(price)
                    .bind(json!({"plan_slug": plan_slug, "tenant_id": tenant_id.to_string()}))
                    .execute(&state.pool)
                    .await?;
                    tracing::info!(tenant = %tenant_id, plan = %plan_slug, session = %provider_session_id, "stripe checkout session created");
                    Ok((
                        StatusCode::CREATED,
                        Json(
                            json!({"url": url, "session_id": provider_session_id, "plan": plan_slug}),
                        ),
                    ))
                }
                Err(e) => {
                    tracing::error!(tenant = %tenant_id, plan = %plan_slug, error = %e, "stripe checkout session FAILED — no charge can occur");
                    Ok((
                        StatusCode::BAD_GATEWAY,
                        Json(
                            json!({"error": "checkout_session_failed", "message": e, "configured": true}),
                        ),
                    ))
                }
            }
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

// ═════════════════════ Stripe: creating a checkout session ═════════════════════
//
// Kept to one outbound call so the whole thing is provable without a live Stripe account: the base
// URL honours `STRIPE_API_BASE` (default = production, so unset behaviour is byte-identical), which
// lets the proof harness point it at a local stub and assert the real request the app builds.
async fn create_stripe_session(
    tenant_id: Uuid,
    user_id: &str,
    plan_id: Uuid,
    plan_name: &str,
    price: f64,
    secret_key: &str,
) -> Result<(String, String), String> {
    let api_base = std::env::var("STRIPE_API_BASE")
        .unwrap_or_else(|_| "https://api.stripe.com".to_string())
        .trim_end_matches('/')
        .to_string();
    let origin = std::env::var("STRIPE_CHECKOUT_ORIGIN")
        .unwrap_or_else(|_| "https://app.funnelswift.net".to_string())
        .trim_end_matches('/')
        .to_string();

    // A monthly plan, so the session is a subscription rather than a one-off charge.
    let form: Vec<(String, String)> = vec![
        ("mode".into(), "subscription".into()),
        ("line_items[0][quantity]".into(), "1".into()),
        ("line_items[0][price_data][currency]".into(), "usd".into()),
        (
            "line_items[0][price_data][unit_amount]".into(),
            format!("{}", (price * 100.0).round() as i64),
        ),
        (
            "line_items[0][price_data][recurring][interval]".into(),
            "month".into(),
        ),
        (
            "line_items[0][price_data][product_data][name]".into(),
            plan_name.to_string(),
        ),
        (
            "success_url".into(),
            format!("{origin}/checkout/success?session_id={{CHECKOUT_SESSION_ID}}"),
        ),
        ("cancel_url".into(), format!("{origin}/checkout/cancelled")),
        ("client_reference_id".into(), tenant_id.to_string()),
        // These two are what the webhook needs to change the right plan for the right customer. They
        // are set HERE and re-read from the (signature-verified) event, never trusted from a caller.
        ("metadata[tenant_id]".into(), tenant_id.to_string()),
        ("metadata[plan_id]".into(), plan_id.to_string()),
        ("metadata[user_id]".into(), user_id.to_string()),
    ];

    let resp = reqwest::Client::new()
        .post(format!("{api_base}/v1/checkout/sessions"))
        .bearer_auth(secret_key)
        .form(&form)
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| format!("could not reach Stripe: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Never echo the key. Stripe's own message is safe and is what an operator needs.
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("Stripe returned {status}"));
        return Err(format!("Stripe refused the session: {msg}"));
    }
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| format!("unreadable Stripe response: {e}"))?;
    let session_id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let url = parsed
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if session_id.is_empty() || url.is_empty() {
        return Err("Stripe accepted the request but returned no session id/url".into());
    }
    Ok((url, session_id))
}

// ═════════════════════ Stripe: verifying a delivery ═════════════════════
//
// The contract is already DECIDED in this fleet (ADASwift ce26bee, ported to WorkflowSwift 6f005e5 and
// missedcallrespondr e408856): CONFIG -> PRESENCE -> AUTHENTICITY -> FRESHNESS, with the freshness arm
// last so an ancient stamp on a FORGED signature still reads as a verification failure. Do not
// re-decide the order or the tolerance.

/// Tolerance for the `t=` stamp, in seconds.
pub const DEFAULT_STRIPE_SIGNATURE_TOLERANCE_SECS: i64 = 300;

/// The configured tolerance, clamped so a typo cannot disable the arm entirely.
fn stripe_tolerance_secs() -> i64 {
    std::env::var("STRIPE_WEBHOOK_TOLERANCE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_STRIPE_SIGNATURE_TOLERANCE_SECS)
        .clamp(30, 86_400)
}

struct StripeRejection {
    reason: &'static str,
    http: StatusCode,
    audit_status: &'static str,
    detail: String,
}

/// Reads `t=`/`v1=` from the `Stripe-Signature` header. BOTH the verifier and the freshness arm read
/// the header through this, so they can never disagree about which bytes were signed.
fn stripe_signature_parts(signature: &str) -> (Option<&str>, Option<&str>) {
    let mut t = None;
    let mut v1 = None;
    for part in signature.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("t=") {
            if t.is_none() {
                t = Some(rest);
            }
        } else if let Some(rest) = part.strip_prefix("v1=") {
            if v1.is_none() {
                v1 = Some(rest);
            }
        }
    }
    (t, v1)
}

/// `None` means "no readable clock in this header".
fn stripe_signature_timestamp(signature: &str) -> Option<i64> {
    stripe_signature_parts(signature)
        .0
        .and_then(|t| t.parse::<i64>().ok())
}

fn ct_eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// HMAC-SHA256 over `"{t}.{body}"`, compared in constant time.
fn verify_stripe_signature(body: &[u8], signature: &str, secret: &str) -> bool {
    let (t, v1) = stripe_signature_parts(signature);
    let (Some(t), Some(v1)) = (t, v1) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(t.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    let Ok(provided) = hex::decode(v1) else {
        return false;
    };
    ct_eq_bytes(expected.as_slice(), &provided)
}

/// The ordered refusal decision. `None` means the delivery is acceptable.
fn stripe_rejection(
    secret: Option<&str>,
    signature: Option<&str>,
    signature_ok: bool,
    signed_at: Option<i64>,
    now: i64,
    tolerance_secs: i64,
) -> Option<StripeRejection> {
    // 1. CONFIG — nothing to verify against.
    if secret.map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Some(StripeRejection {
            reason: "stripe_webhook_secret_not_configured",
            http: StatusCode::SERVICE_UNAVAILABLE,
            audit_status: "not_configured",
            detail: "No Stripe webhook signing secret is configured, so a delivery cannot be verified. Add it in the admin panel; nothing was changed.".into(),
        });
    }
    // 2. PRESENCE — a delivery with no signature at all.
    if signature.map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Some(StripeRejection {
            reason: "stripe_signature_missing",
            http: StatusCode::UNAUTHORIZED,
            audit_status: "signature_failed",
            detail: "The Stripe-Signature header is missing. Nothing was changed.".into(),
        });
    }
    // 3. AUTHENTICITY — wrong secret or tampered body.
    if !signature_ok {
        return Some(StripeRejection {
            reason: "stripe_signature_verification_failed",
            http: StatusCode::UNAUTHORIZED,
            audit_status: "signature_failed",
            detail: "The signature does not match this delivery. Nothing was changed.".into(),
        });
    }
    // 4. FRESHNESS — last, so a forged AND ancient pair reads as a verification failure.
    if signed_at
        .map(|t| (now - t).abs() > tolerance_secs)
        .unwrap_or(true)
    {
        return Some(StripeRejection {
            reason: "stripe_signature_timestamp_out_of_tolerance",
            http: StatusCode::SERVICE_UNAVAILABLE,
            audit_status: "signature_failed",
            detail: format!(
                "The signed timestamp is outside the {tolerance_secs}s tolerance (a replayed delivery). Nothing was changed."
            ),
        });
    }
    None
}

/// Record a delivery. Returns `true` when this is the FIRST time this event has been seen.
async fn record_event(
    state: &AppState,
    event_id: &str,
    event_type: &str,
    status: &str,
    error: Option<&str>,
    tenant_id: Option<Uuid>,
    payload: &Value,
) -> Result<bool, sqlx::Error> {
    // The HTTP code is derived from the status rather than passed in, so the audit row and the
    // response can never disagree about what happened.
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO payment_webhook_events (provider_type, event_id, event_type, status, http_status, error_message, tenant_id, payload, processed_at)
         VALUES ('stripe', $1, $2, $3,
                 CASE $3 WHEN 'processed' THEN 200 WHEN 'ignored' THEN 200 WHEN 'duplicate' THEN 200
                         WHEN 'signature_failed' THEN 401 WHEN 'not_configured' THEN 503 ELSE NULL END,
                 $4, $5, $6, CASE WHEN $3 IN ('processed','ignored','duplicate') THEN NOW() ELSE NULL END)
         ON CONFLICT (provider_type, event_id) DO NOTHING
         RETURNING id",
    )
    .bind(event_id)
    .bind(event_type)
    .bind(status)
    .bind(error)
    .bind(tenant_id)
    .bind(payload)
    .fetch_optional(&state.pool)
    .await?;
    Ok(inserted.is_some())
}

/// The free plan that belongs to the same product side as `current`, used when a subscription ends.
async fn free_plan_for(state: &AppState, current: Uuid) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT f.id FROM plans f
          WHERE COALESCE(f.price, 0) = 0
            AND f.side = (SELECT side FROM plans WHERE id = $1)
          ORDER BY f.created_at LIMIT 1",
    )
    .bind(current)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
}

/// How long ago the delivery was signed, in whole seconds — reported so a refusal can be explained.
fn stripe_signature_age(signature: &str, now: i64) -> Option<i64> {
    stripe_signature_timestamp(signature).map(|t| now - t)
}

/// POST /api/v1/webhooks/stripe
///
/// Unauthenticated by necessity (Stripe cannot present a session token) and therefore verified by
/// signature instead. Everything it can change goes through `plan_handler::set_active_plan`, which is
/// also where a referred customer's movement is DATED and the affiliate's commission is settled — so
/// paying here is exactly what credits the affiliate.
pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<(StatusCode, Json<Value>)> {
    let signature = headers
        .get("Stripe-Signature")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let now = chrono::Utc::now().timestamp();

    // Which provider's secret? There is no session to name the tenant, so the delivery is matched to
    // an ACTIVE Stripe provider by trying its secret — the tenant is then established by the delivery
    // that actually verifies, never by a field the caller could have written.
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT tenant_id, webhook_secret FROM payment_providers
          WHERE provider_type = 'stripe' AND is_active = true
            AND webhook_secret IS NOT NULL AND webhook_secret <> ''
          ORDER BY created_at LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut matched: Option<(Uuid, String)> = None;
    let mut any_secret = false;
    if let Some(sig) = signature.as_deref() {
        for (tenant_id, stored) in &rows {
            let Ok(plain) =
                crate::security::provider_key_crypto::decrypt_from_storage(&state.pool, stored)
                    .await
            else {
                continue;
            };
            if plain.trim().is_empty() {
                continue;
            }
            any_secret = true;
            if matched.is_none() && verify_stripe_signature(&body, sig, &plain) {
                matched = Some((*tenant_id, plain));
            }
        }
    }
    // A configured secret whose ciphertext cannot be read still counts as configured, so the refusal
    // below reports "unreadable" rather than "not configured" — those need different fixes.
    if !any_secret && !rows.is_empty() {
        any_secret = true;
    }

    let ok = matched.is_some();
    let secret_for_decision: Option<&str> = if rows.is_empty() {
        None
    } else {
        Some("configured")
    };
    if let Some(rejection) = stripe_rejection(
        secret_for_decision,
        signature.as_deref(),
        ok,
        stripe_signature_timestamp(signature.as_deref().unwrap_or("")),
        now,
        stripe_tolerance_secs(),
    ) {
        // An unreadable secret is the one CONFIG case that is not "nothing configured".
        let reason = if !rows.is_empty() && !any_secret {
            "stripe_webhook_secret_unreadable"
        } else {
            rejection.reason
        };
        let audit_id = format!("rejected:{}", &hex::encode(Sha256::digest(&body[..]))[..32]);
        let parsed: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let _ = record_event(
            &state,
            &audit_id,
            parsed
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown"),
            rejection.audit_status,
            Some(reason),
            None,
            &parsed,
        )
        .await;
        if rejection.reason == "stripe_signature_timestamp_out_of_tolerance" {
            tracing::error!(
                signed_at = ?stripe_signature_timestamp(signature.as_deref().unwrap_or("")),
                age = ?stripe_signature_age(signature.as_deref().unwrap_or(""), now).map(|a| format!("{a}s")),
                tolerance = %format!("{}s", stripe_tolerance_secs()),
                "Stripe webhook REFUSED: {}",
                rejection.reason
            );
        } else {
            tracing::error!(reason = %reason, "Stripe webhook REFUSED — nothing was changed");
        }
        return Ok((
            rejection.http,
            Json(
                json!({"error": reason, "message": rejection.detail, "configured": !rows.is_empty()}),
            ),
        ));
    }

    let (tenant_id, _secret) = matched.expect("matched is Some when no rejection was returned");
    let event: Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("unreadable event body: {e}")))?;
    let event_id = event
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event_type = event
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if event_id.is_empty() {
        return Err(AppError::BadRequest("event has no id".into()));
    }
    let object = event
        .pointer("/data/object")
        .cloned()
        .unwrap_or_else(|| json!({}));

    // Idempotency: the provider retries for days, so the FIRST delivery wins and every later one is
    // recorded as a duplicate without touching a plan.
    let first_time = record_event(
        &state,
        &event_id,
        &event_type,
        "processing",
        None,
        Some(tenant_id),
        &event,
    )
    .await?;
    if !first_time {
        return Ok((
            StatusCode::OK,
            Json(json!({"status": "duplicate", "event_id": event_id})),
        ));
    }

    let meta_tenant = object
        .pointer("/metadata/tenant_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or_else(|| {
            object
                .get("client_reference_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
        })
        .unwrap_or(tenant_id);
    let meta_plan = object
        .pointer("/metadata/plan_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let outcome: Result<&str, String> = match event_type.as_str() {
        "checkout.session.completed" => match meta_plan {
            Some(plan_id) => match crate::handlers::plan_handler::set_active_plan(
                &state.pool,
                meta_tenant,
                plan_id,
            )
            .await
            {
                Ok((_, slug)) => {
                    let _ = sqlx::query(
                        "UPDATE checkout_sessions SET status = 'completed', webhook_event_id = $1, webhook_received_at = NOW(), updated_at = NOW()
                          WHERE provider_type = 'stripe' AND provider_session_id = $2",
                    )
                    .bind(&event_id)
                    .bind(object.get("id").and_then(|v| v.as_str()).unwrap_or(""))
                    .execute(&state.pool)
                    .await;
                    tracing::info!(
                        event = %event_id, tenant = %meta_tenant, plan = %slug,
                        "STRIPE: payment completed — plan activated, movement dated, affiliate settled"
                    );
                    Ok("plan_activated")
                }
                Err(e) => Err(format!("could not activate the plan: {e}")),
            },
            None => Err("completed session carried no plan_id in its metadata".into()),
        },
        // Stripe tells us a subscription ended: the customer has left a paying plan. This is the
        // downgrade the affiliate's dashboard has to see, dated, or their earnings keep looking real.
        "customer.subscription.deleted" | "customer.subscription.paused" => {
            let current: Option<Uuid> = sqlx::query_scalar(
                "SELECT tps.plan_id FROM tenant_plan_subscriptions tps
                  WHERE tps.tenant_id = $1 AND tps.status = 'active'
                  ORDER BY tps.start_date DESC LIMIT 1",
            )
            .bind(meta_tenant)
            .fetch_optional(&state.pool)
            .await?;
            let free = match current {
                Some(c) => free_plan_for(&state, c).await,
                None => None,
            };
            match free {
                Some(free_id) => {
                    match crate::handlers::plan_handler::set_active_plan(
                        &state.pool,
                        meta_tenant,
                        free_id,
                    )
                    .await
                    {
                        Ok((_, slug)) => {
                            tracing::info!(event = %event_id, tenant = %meta_tenant, plan = %slug,
                                "STRIPE: subscription ended — downgraded, movement dated, commission reversed");
                            Ok("plan_downgraded")
                        }
                        Err(e) => Err(format!("could not downgrade: {e}")),
                    }
                }
                None => Err("no free plan to fall back to".into()),
            }
        }
        "checkout.session.expired" => {
            let _ = sqlx::query(
                "UPDATE checkout_sessions SET status = 'expired', webhook_event_id = $1, webhook_received_at = NOW(), updated_at = NOW()
                  WHERE provider_type = 'stripe' AND provider_session_id = $2 AND status = 'pending'",
            )
            .bind(&event_id)
            .bind(object.get("id").and_then(|v| v.as_str()).unwrap_or(""))
            .execute(&state.pool)
            .await;
            Ok("session_expired")
        }
        _ => Ok("ignored"),
    };

    match outcome {
        Ok(what) => {
            let _ = sqlx::query(
                "UPDATE payment_webhook_events SET status = $1, http_status = 200, processed_at = NOW()
                  WHERE provider_type = 'stripe' AND event_id = $2",
            )
            .bind(if what == "ignored" { "ignored" } else { "processed" })
            .bind(&event_id)
            .execute(&state.pool)
            .await;
            Ok((
                StatusCode::OK,
                Json(json!({"status": what, "event_id": event_id})),
            ))
        }
        Err(e) => {
            tracing::error!(event = %event_id, event_type = %event_type, error = %e, "Stripe webhook could NOT be applied");
            let _ = sqlx::query(
                "UPDATE payment_webhook_events SET status = 'error', http_status = 500, error_message = $1, processed_at = NOW()
                  WHERE provider_type = 'stripe' AND event_id = $2",
            )
            .bind(&e)
            .bind(&event_id)
            .execute(&state.pool)
            .await;
            // A 500 tells Stripe to retry, which is right for a transient failure.
            Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "webhook_not_applied", "message": e})),
            ))
        }
    }
}
