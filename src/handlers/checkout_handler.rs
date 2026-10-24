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
        String,
    )> = sqlx::query_as(
        "SELECT id, tenant_id, provider_type, COALESCE(api_key,''), is_active, created_at,
                (webhook_secret IS NOT NULL AND webhook_secret <> '') AS has_webhook_secret,
                COALESCE(webhook_secret,'')
           FROM payment_providers WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    // payment_providers.api_key is CIPHERTEXT at rest (src/security/provider_key_crypto.rs), so
    // the read-back is decrypted HERE and only a mask of the DECRYPTED value is returned. Before
    // kanban t_63840ff2 this endpoint put the raw secret key in the response body.
    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for (
        id,
        tenant_id,
        provider_type,
        stored,
        is_active,
        created_at,
        has_webhook_secret,
        stored_secret,
    ) in rows
    {
        let plain =
            crate::security::provider_key_crypto::decrypt_from_storage(&state.pool, stored.trim())
                .await?;
        out.push(json!({
            "id": id,
            "tenant_id": tenant_id,
            "provider_type": provider_type,
            "api_key_masked": crate::security::provider_key_crypto::mask(&plain),
            // At-rest posture of the SAVED credential (kanban t_d1fcda7e family). The mask above is
            // computed from the DECRYPTED value, so it cannot tell a sealed row from a legacy
            // plaintext one; this is measured off the ciphertext COLUMN through the SAME
            // `is_encrypted` predicate the write path seals with. Every sibling app's Payment
            // providers panel already reports it and this one did not.
            "api_key_set": !stored.trim().is_empty(),
            "api_key_sealed": crate::security::provider_key_crypto::is_encrypted(stored.trim()),
            // Whether a webhook signing secret is stored — never the secret itself. The receiver
            // cannot verify a single delivery without one, so a provider row missing it must SAY so
            // here rather than look complete.
            "webhook_secret_set": has_webhook_secret,
            "webhook_secret_sealed": crate::security::provider_key_crypto::is_encrypted(
                stored_secret.trim(),
            ),
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
    // `payment_providers.tenant_id` REFERENCES `tenants(id)`. This upsert is the only writer the
    // operator's Provider Keys screen drives, and that screen runs on the PLATFORM ADMIN role, which
    // is exempt from `global_auth`'s workspace-existence gate — so the workspace has to be checked
    // HERE, before the INSERT can reach the constraint (`tenant_handler::require_existing_workspace`
    // carries the measurement and the wording).
    let tenant_id =
        crate::handlers::tenant_handler::require_existing_workspace(&state.pool, &auth.tenant_id)
            .await?;
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
    let result =
        sqlx::query("DELETE FROM payment_providers WHERE provider_type = $1 AND tenant_id = $2")
            .bind(&provider_type)
            .bind(tenant_id)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Payment provider not found".into()));
    }
    Ok(Json(json!({"message": "Provider deleted"})))
}
/// WHICH payment provider funds a PLAN purchase (kanban t_ff1f0d6b, measured 2026-10-15).
///
/// A plan is sold BY the platform TO a workspace, so the merchant account is the platform's. This
/// used to read the CALLER's own row only, while the only shipped writer of the table is the
/// operator's own admin-panel entry (`www-admin` → Provider Keys / Payment providers, which binds
/// the signed-in account's tenant id). The consequence was measured live on the whole install:
/// `payment_providers` held **0 rows**, `POST /api/v1/checkout/create` answered 503
/// `payment_provider_not_configured` for the ladder on a real workspace token, and putting the
/// operator's Stripe key in the admin panel could NOT have changed that — every plan purchase would
/// still have looked for a row owned by the BUYING workspace. A ladder that cannot be bought is the
/// defect this function closes.
///
/// Resolution order, lowest-surprise first:
///   1. the buying workspace's OWN active row — unchanged behaviour, so a workspace that brings its
///      own merchant account (BYOK) keeps paying through it, and nothing existing moves;
///   2. otherwise the OLDEST active row owned by a tenant that has an `admin`-role user — the app's
///      own definition of the platform operator (`auth/middleware.rs`, `is_admin = role == "admin"`),
///      i.e. the key the operator entered in the admin panel;
///   3. otherwise `None` -> the caller answers 503 naming exactly where the key belongs.
///
/// The admin-role guard on arm 2 is load-bearing: without it ANY workspace could register, save its
/// own Stripe key and have every customer's plan purchase on the install charged to that workspace's
/// account. Registration mints `role = 'user'`, so an ordinary tenant cannot reach arm 2 by itself.
/// The money always lands on the resolved owner; the PLAN always changes on the BUYER, because the
/// webhook grants to `metadata[tenant_id]` (written by `create_stripe_session` from the caller, never
/// from the provider) — see `stripe_webhook`.
async fn resolve_plan_provider(
    state: &AppState,
    tenant_id: Uuid,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    let own: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT tenant_id, provider_type FROM payment_providers \
         WHERE tenant_id = $1 AND is_active = true ORDER BY created_at LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    if own.is_some() {
        return Ok(own);
    }
    sqlx::query_as(
        "SELECT pp.tenant_id, pp.provider_type FROM payment_providers pp \
         WHERE pp.is_active = true \
           AND EXISTS (SELECT 1 FROM users u WHERE u.tenant_id = pp.tenant_id AND u.role = 'admin') \
         ORDER BY pp.created_at LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await
}

/// POST /api/v1/checkout/create
///
/// A REAL Stripe checkout, or an explicit refusal. This doc comment used to say "there is no payment
/// provider integration in FunnelSwift yet (which provider is canonical — Stripe / Mintbird /
/// Groovesell — is a product decision)"; that stopped being true when [`create_stripe_session`] was
/// written, and a comment that contradicts the code is how the next reader mis-plans. The arms now:
///
/// * no active `payment_providers` row for the CALLER and none owned by a platform-operator account
///   -> 503 `payment_provider_not_configured`, naming where the key belongs
///   ([`resolve_plan_provider`] documents the two-arm resolution: the buyer's own merchant account
///   first, then the operator's — a plan is sold by the platform TO a workspace)
/// * a provider row that is not Stripe -> 501 `checkout_not_implemented`, naming the provider
/// * a Stripe row whose key cannot be read -> 503 `payment_provider_not_configured`
/// * Stripe ACCEPTED the session -> 201 `{url, session_id, plan}` + a `checkout_sessions` row
/// * Stripe REFUSED / unreachable -> **424** `checkout_session_failed` carrying Stripe's own message
///   (never 502 — the CDN replaces such a body; see the arm's comment below)
///
/// It still never fabricates a URL and never charges: the only session row written is one Stripe
/// itself returned.
pub async fn create_checkout_session(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();

    // YEARLY CHECKOUT (2026-10-24). The public pricing page sells every paid tier with an annual
    // price ("$9/month or $89 a year (two months free)") and `plans.annual_price` has carried those
    // numbers all along, but this handler only ever built a MONTHLY Stripe session — so the yearly
    // offer the site advertises could not be bought anywhere in the product. `billing_cycle` is
    // optional: absent means "month", which is byte-identical to the old behaviour, and a caller
    // who asks for a year on a plan that has no yearly price is refused BY NAME instead of being
    // quietly charged the monthly rate.
    let billing_cycle = payload["billing_cycle"]
        .as_str()
        .unwrap_or("month")
        .trim()
        .to_lowercase();
    let yearly = match billing_cycle.as_str() {
        "" | "month" | "monthly" => false,
        "year" | "yearly" | "annual" => true,
        other => {
            return Err(AppError::Validation(format!(
                "Unknown billing_cycle '{other}' — use 'month' or 'year'."
            )))
        }
    };

    let configured = resolve_plan_provider(&state, tenant_id).await?;

    match configured {
        None => {
            // A REFUSED upgrade is still demand (2026-10-19). Nobody was charged and no provider
            // session exists, but the workspace asked for a named plan — so record the attempt in
            // `checkout_sessions` with a status that says exactly why it stopped. The workspace's
            // own Checkout Sessions screen is the history of what was attempted, and support can
            // read which plan was wanted instead of guessing from a 503 in a log line.
            let plan_slug = payload["plan_slug"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            let plan: Option<(Uuid, String, f64, f64)> = sqlx::query_as(
                "SELECT id, name, COALESCE(price, 0)::float8, COALESCE(annual_price, 0)::float8 \
                 FROM plans WHERE slug = $1",
            )
            .bind(&plan_slug)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
            let user_id = Uuid::parse_str(&auth.user_id).ok();
            // `amount` is NUMERIC; bind the price as text and cast, so the type is never in doubt.
            // The figure recorded is the one the buyer ASKED for, so a refused yearly request reads
            // back as the yearly amount rather than as the monthly rate.
            let amount_text = plan
                .as_ref()
                .map(|p| format!("{:.2}", if yearly { p.3 } else { p.2 }));
            // ── A PLAN CAN BE SOLD WITHOUT AN API KEY (2026-10-24) ───────────────────────────
            // `plans.purchase_url` is a hosted payment page the OPERATOR pastes into the plan (a
            // Stripe Payment Link, a PayPal button, anything that takes the money). The column has
            // had a writer since the plans table shipped — `create_plan`/`update_plan` both bind it,
            // and the console form never offered it — but NOTHING read it at the till, so on an
            // install with no provider key (`payment_providers` is EMPTY in all five apps that hold
            // the table, card t_50052174) every upgrade died here even when the operator already had
            // a working way to be paid. A plan that carries a link is SELLABLE: hand the buyer to
            // it, and STILL file the ask, because a hosted link tells this installation nothing —
            // the workspace is seated when the operator resolves the request, never by the redirect.
            let payment_link = match plan.as_ref() {
                Some(_) => sqlx::query_scalar::<_, Option<String>>(
                    "SELECT NULLIF(BTRIM(purchase_url), '') FROM plans WHERE slug = $1",
                )
                .bind(&plan_slug)
                .fetch_optional(&state.pool)
                .await
                .unwrap_or(None)
                .flatten()
                .filter(|u| is_hosted_payment_url(u)),
                None => None,
            };
            if let Some(url) = payment_link {
                // The history row says what actually happened: the buyer was handed to the
                // operator's own payment page. `provider_missing` would read as "nothing happened".
                let handed_over = sqlx::query(
                    "INSERT INTO checkout_sessions \
                       (tenant_id, user_id, provider_type, purchasable_type, purchasable_id, amount, \
                        currency, status, metadata) \
                     VALUES ($1, $2, 'link', 'plan', $3, COALESCE($4::numeric, 0), 'USD', \
                        'payment_link', $5)",
                )
                .bind(tenant_id)
                .bind(user_id)
                .bind(plan.as_ref().map(|p| p.0))
                .bind(amount_text)
                .bind(json!({
                    "plan_slug": plan_slug,
                    "billing_cycle": if yearly { "year" } else { "month" },
                    "reason": "hosted_payment_link"
                }))
                .execute(&state.pool)
                .await;
                if let Err(e) = handed_over {
                    tracing::error!(
                        "checkout/create: could not record the payment-link hand-off: {e:?}"
                    );
                }
                let plan_request = match plan.as_ref() {
                    Some((_, name, price, annual_price)) => {
                        let asked_for = if yearly { *annual_price } else { *price };
                        record_upgrade_request(
                            &state.pool,
                            UpgradeAsk {
                                tenant_id,
                                user_id,
                                requested_by: &auth.email,
                                plan_slug: &plan_slug,
                                plan_name: name,
                                plan_price: asked_for,
                                payment_link: Some(url.as_str()),
                            },
                        )
                        .await
                    }
                    None => None,
                };
                return Ok((
                    StatusCode::OK,
                    Json(json!({
                        "status": "payment_link",
                        "url": url,
                        "checkout_url": url,
                        "plan": {
                            "id": plan.as_ref().map(|p| p.0.to_string()),
                            "slug": plan_slug,
                            "billing_cycle": if yearly { "year" } else { "month" },
                        },
                        "request_recorded": plan_request.is_some(),
                        "plan_request": plan_request,
                        "message": "Opening the plan's payment page now. Nothing has been charged \
                                    yet — once the payment lands, the platform team switches this \
                                    workspace onto the plan."
                    })),
                ));
            }
            let recorded = sqlx::query(
                "INSERT INTO checkout_sessions \
                   (tenant_id, user_id, provider_type, purchasable_type, purchasable_id, amount, \
                    currency, status, metadata) \
                 VALUES ($1, $2, 'none', 'plan', $3, COALESCE($4::numeric, 0), 'USD', \
                    'provider_missing', $5)",
            )
            .bind(tenant_id)
            .bind(user_id)
            .bind(plan.as_ref().map(|p| p.0))
            .bind(amount_text)
            .bind(json!({
                "plan_slug": plan_slug,
                "billing_cycle": if yearly { "year" } else { "month" },
                "reason": "payment_provider_not_configured"
            }))
            .execute(&state.pool)
            .await;
            if let Err(e) = recorded {
                tracing::error!("checkout/create: could not record the refused attempt: {e:?}");
            }
            // THE ASK, WHERE THE OPERATOR CAN ACT ON IT (2026-10-23; parity with ADASwift,
            // WorkflowSwift and IncentiveSwift). `checkout_sessions` is a HISTORY screen: it shows
            // that a workspace pressed Upgrade, but closing the request there means hand-assigning
            // a plan. The operator's Plan requests queue reads `plan_requests` and its Resolve arm
            // switches the workspace onto the tier, so the ask is filed THERE, server-side, deduped
            // on one open request per (tenant, plan_slug) -- the same key `plan_request_handler`'s
            // customer door uses. Before this the row only existed if the console's own follow-up
            // press ran, so a buyer who closed the tab had asked for nothing. Inserted directly
            // rather than through that handler so no receipt mail is queued: nothing was charged.
            let plan_request = match plan.as_ref() {
                Some((_, name, price, annual_price)) => {
                    let asked_for = if yearly { *annual_price } else { *price };
                    record_upgrade_request(
                        &state.pool,
                        UpgradeAsk {
                            tenant_id,
                            user_id,
                            requested_by: &auth.email,
                            plan_slug: &plan_slug,
                            plan_name: name,
                            plan_price: asked_for,
                            payment_link: None,
                        },
                    )
                    .await
                }
                None => None,
            };
            // A buyer must never be told their ask was recorded unless it WAS (2026-10-24). The
            // sentence below promises the platform team will see the request, and the console reads
            // this flag to decide whether to offer its own fallback request button — so the flag is
            // the RECORD's outcome, never an assumption. Measured before this: an upgrade carrying a
            // plan slug that matches no row answered `request_recorded: true, plan_request: null`
            // while `plan_requests` gained nothing (count 1 -> 1) — a promise with no write behind
            // it. Same invariant ADASwift already holds: "promising a row that does not exist is the
            // same class of lie as a 200 with no write".
            let request_recorded = plan_request.is_some();
            // The buyer's sentence follows the same fact: the recording clause is only spoken when a
            // row exists. The facts a buyer needs come first (nothing was charged; whether their ask
            // was taken), the operator's setup detail stays, in parentheses, for whoever reads it next.
            let message = if request_recorded {
                "No charge was made — nothing was sent to a payment provider, so you cannot be billed. Upgrades are not switched on for this installation yet: your request has been recorded for the platform team, who can activate this plan for your workspace by hand. (Operator: add Stripe's keys in the admin panel under Provider Keys / Payment providers on the operator's own account; that key then funds every workspace's plan purchase, and a workspace that brings its own Stripe account is charged through its own.)"
            } else {
                "No charge was made — nothing was sent to a payment provider, so you cannot be billed. Upgrades are not switched on for this installation yet, and the plan you picked does not match a plan on this installation, so nothing was recorded: reload the page and press Upgrade again, and the platform team can still activate a plan for your workspace by hand. (Operator: add Stripe's keys in the admin panel under Provider Keys / Payment providers on the operator's own account; that key then funds every workspace's plan purchase, and a workspace that brings its own Stripe account is charged through its own.)"
            };
            tracing::error!(
                "checkout/create REFUSED: no active payment provider for tenant {tenant_id}, and \
                 no active provider owned by a platform-operator account either"
            );
            Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "payment_provider_not_configured",
                    // BUYER-FIRST (2026-10-22). This sentence is read by the CUSTOMER on the upgrade
                    // screen (`upgrade.js` paints the server's own `message` into the error panel), and
                    // it used to open by telling that customer how the platform operator configures
                    // Stripe in his admin panel — an instruction addressed to somebody else, on the one
                    // screen where a buyer is trying to give us money. The two facts a buyer needs come
                    // first (nothing was charged; the attempt was recorded and can be acted on), the
                    // operator's setup detail stays, in parentheses, for whoever reads it next.
                    "message": message,
                    "configured": false,
                    "request_recorded": request_recorded,
                    "plan_request": plan_request
                })),
            ))
        }
        Some((provider_tenant, provider_type)) => {
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
            let plan: Option<(Uuid, String, f64, f64)> = sqlx::query_as(
                "SELECT id, name, COALESCE(price, 0)::float8, COALESCE(annual_price, 0)::float8 \
                 FROM plans WHERE slug = $1",
            )
            .bind(&plan_slug)
            .fetch_optional(&state.pool)
            .await?;
            let Some((plan_id, plan_name, price, annual_price)) = plan else {
                return Err(AppError::NotFound("Unknown plan".into()));
            };
            if price <= 0.0 {
                return Err(AppError::Validation(
                    "That plan costs nothing — there is nothing to check out.".into(),
                ));
            }
            // A yearly ask on a tier with no yearly price is refused by name: falling back to the
            // monthly amount would charge the buyer something they did not agree to.
            let amount = if yearly { annual_price } else { price };
            if amount <= 0.0 {
                return Err(AppError::Validation(
                    "This plan has no yearly price, so it can only be bought monthly.".into(),
                ));
            }
            let interval = if yearly { "year" } else { "month" };
            // The key comes from the RESOLVED owner's row (see `resolve_plan_provider`): a workspace
            // that brought its own merchant account pays through it, everyone else through the
            // platform operator's. Logged when the two differ, so "which account was charged" is
            // answerable from the log line rather than by re-deriving the resolution.
            if provider_tenant != tenant_id {
                tracing::info!(
                    buyer = %tenant_id, provider_owner = %provider_tenant,
                    "checkout/create: charging through the platform account (buyer has no provider of its own)"
                );
            }
            let key: Option<String> = sqlx::query_scalar(
                "SELECT api_key FROM payment_providers WHERE tenant_id = $1 AND provider_type = 'stripe' AND is_active = true LIMIT 1",
            )
            .bind(provider_tenant)
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
                amount,
                interval,
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
                    .bind(amount)
                    .bind(json!({
                        "plan_slug": plan_slug,
                        "tenant_id": tenant_id.to_string(),
                        "billing_cycle": interval
                    }))
                    .execute(&state.pool)
                    .await?;
                    tracing::info!(tenant = %tenant_id, plan = %plan_slug, session = %provider_session_id, "stripe checkout session created");
                    Ok((
                        StatusCode::CREATED,
                        Json(json!({
                            "url": url,
                            "session_id": provider_session_id,
                            "plan": plan_slug,
                            "billing_cycle": interval,
                            "amount": amount
                        })),
                    ))
                }
                Err(e) => {
                    tracing::error!(tenant = %tenant_id, plan = %plan_slug, error = %e, "stripe checkout session FAILED — no charge can occur");
                    // 424 FAILED DEPENDENCY, never 502 (kanban work 2026-10-15). Cloudflare —
                    // which fronts app.funnelswift.net — REPLACES the body of an origin 502 (and
                    // 504) with its own page: measured live, this arm answered the origin
                    // `502 application/json {"message":"Stripe refused the session: Invalid API
                    // Key provided: sk_test_…3219"}` while the EDGE answered the same POST
                    // `502 text/plain` with the body `error code: 502`. The Upgrade pane's fetch
                    // parses the reply as JSON, so it threw and the customer saw NOTHING at all
                    // when a payment attempt failed. 424 is the exact meaning here (the action
                    // depended on another action and that action failed) and, being a 4xx, the CDN
                    // passes the body through verbatim. Same decision MissedCall Respondr's SMS
                    // path and this app's CoreSwift integration already carry.
                    Ok((
                        StatusCode::FAILED_DEPENDENCY,
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
///
/// `:id` accepts BOTH references for the SAME checkout, because the two ends of this journey speak
/// different languages (kanban t_2d410c1f): our own `checkout_sessions.id` (a uuid — the form this
/// endpoint was written against) AND the payment provider's session id, which is what Stripe
/// substitutes into the `{CHECKOUT_SESSION_ID}` placeholder of the `success_url` THIS repo builds
/// (`create_stripe_session`) — so the buyer's real return URL carries `cs_live_…`, never a uuid.
/// Resolving only the uuid meant the thank-you page could only ever answer "we could not find that
/// checkout", even for a payment that had genuinely gone through. Both values are unguessable 20+
/// character tokens and the reply still carries no tenant/user/provider field, so the lookup stays
/// exactly as id-scoped as it was.
pub async fn get_checkout_session_public(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let as_uuid = Uuid::parse_str(&session_id).ok();

    let row = sqlx::query(
        r#"-- A non-uuid reference is bound as NULL on the first arm, which matches nothing, so the
           -- provider-session-id arm carries those lookups on its own.
           SELECT cs.id, cs.purchasable_type, cs.amount::text AS amount,
                  cs.currency, cs.status, cs.created_at,
                  p.name AS plan_name
           FROM checkout_sessions cs
           LEFT JOIN plans p
                  ON cs.purchasable_type = 'plan' AND p.id = cs.purchasable_id
           WHERE cs.id = $1
              OR (cs.provider_session_id <> '' AND cs.provider_session_id = $2)
           ORDER BY (cs.id = $1) DESC NULLS LAST, cs.created_at DESC
           LIMIT 1"#,
    )
    .bind(as_uuid)
    .bind(&session_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        tracing::warn!("checkout session lookup miss: {session_id}");
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({
                "found": false,
                "error": "checkout_session_not_found",
                "note": "No checkout session matches that reference — we look it up by our own checkout id and by the payment provider's session id. If you just paid, your provider receipt is authoritative; contact support if your plan is not active."
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
    // What the operator's screen has to answer is "which plan did this workspace try to buy, and
    // what happened to it" — not a list of uuids. `purchasable_id` alone cannot answer that, so the
    // plan's own name/slug are joined in here, and the refusal's cause is lifted out of `metadata`
    // (the `provider_missing` arm writes `reason = payment_provider_not_configured`). A session whose
    // purchasable row is gone keeps its row and reads as unnamed rather than disappearing.
    let rows = sqlx::query(
        r#"SELECT cs.id, cs.provider_type, cs.purchasable_type, cs.purchasable_id::text,
                  cs.amount::text, cs.currency, cs.status, cs.provider_session_id, cs.created_at,
                  cs.metadata, p.name AS plan_name, p.slug AS plan_slug
           FROM checkout_sessions cs
           LEFT JOIN plans p
                  ON cs.purchasable_type = 'plan' AND p.id = cs.purchasable_id
           WHERE cs.tenant_id = $1
           ORDER BY cs.created_at DESC
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
                // What was being bought, named. `None` for a non-plan purchase or a plan row that
                // has since been deleted — the screen renders that as a dash, never as a bare uuid.
                "plan_name": r.try_get::<Option<String>,_>("plan_name").unwrap_or(None),
                "plan_slug": r.try_get::<Option<String>,_>("plan_slug").unwrap_or(None),
                // The recorded cause, when the arm that wrote the row left one.
                "reason": r
                    .try_get::<Option<Value>,_>("metadata")
                    .unwrap_or(None)
                    .and_then(|m| m.get("reason").and_then(|v| v.as_str()).map(|s| s.to_string())),
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
    interval: &str,
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

    // A RECURRING plan, so the session is a subscription rather than a one-off charge. `interval`
    // is the term the buyer chose: "month" (the only term this app could sell before 2026-10-24) or
    // "year", which is the annual price the public pricing page advertises.
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
            interval.to_string(),
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

// ═════════════════════ Verifying the operator's payment key ═════════════════════
//
// kanban t_6a192c06. Until this route existed, the fleet's ONLY way to learn that a payment key was
// wrong, truncated or from another account was for a CUSTOMER to press Upgrade and receive the 424
// that carries Stripe's refusal. `payment_providers` is empty in every app (measured 2026-10-15), so
// every purchase refuses `payment_provider_not_configured` — and an operator pasting a key had no way
// to check it before it mattered. This route makes the key verifiable at the moment it is pasted.
//
// The check is a READ: Stripe's `GET /v1/balance`. It moves no money and creates no Stripe object.
//
// One body shape, `{ "api_key": "<key>" }`, and both arms are useful:
//   * `api_key` non-empty -> test THAT key. It is never stored, never logged and never echoed (only
//     [`crate::security::provider_key_crypto::mask`]-ed), so a typo cannot overwrite a working
//     credential the way a Save would. This is the button beside the Save form.
//   * `api_key` empty/absent -> test the key STORED for this workspace, resolved exactly as a purchase
//     resolves it ([`resolve_plan_provider`]: the workspace's own active row first, then the platform
//     operator's). This is the per-row button.
//
// Every refusal is a 4xx class JSON body, never a 502: a CDN replaces a 502 body and the operator
// would lose the reason, which is the same rule [`create_checkout_session`] follows.
pub async fn test_payment_provider(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(provider_type): Path<String>,
    payload: Option<Json<Value>>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let provider = provider_type.trim().to_ascii_lowercase();
    let supplied = payload
        .as_ref()
        .and_then(|j| j.0.get("api_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let (key, source) = if !supplied.is_empty() {
        (supplied, "supplied")
    } else {
        let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
        let resolved = resolve_plan_provider(&state, tenant_id).await?;
        let Some((row_tenant, row_provider)) = resolved else {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "ok": false,
                    "error": "payment_provider_not_configured",
                    "reason": "payment_provider_not_configured",
                    "message": "No payment provider is stored for this workspace or for the platform, so there is no stored key to test. Paste a key and press Test key, or save one first.",
                })),
            ));
        };
        if provider != row_provider.to_ascii_lowercase() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({
                    "ok": false,
                    "error": "payment_provider_not_stored",
                    "reason": "payment_provider_not_stored",
                    "message": format!(
                        "The stored provider for this workspace is '{row_provider}', not '{provider}', so there is no '{provider}' key to test."
                    ),
                })),
            ));
        }
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT api_key FROM payment_providers \
             WHERE tenant_id = $1 AND provider_type = $2 AND is_active = true \
             ORDER BY created_at LIMIT 1",
        )
        .bind(row_tenant)
        .bind(&row_provider)
        .fetch_optional(&state.pool)
        .await?;
        match stored
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            Some(s) => {
                let plain =
                    crate::security::provider_key_crypto::decrypt_from_storage(&state.pool, &s)
                        .await?;
                (plain, "stored")
            }
            None => {
                return Ok((
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "ok": false,
                        "error": "payment_provider_not_configured",
                        "reason": "payment_provider_not_configured",
                        "message": format!(
                            "The stored '{row_provider}' provider row carries no key, so there is nothing to test. Paste the key and press Save first."
                        ),
                    })),
                ));
            }
        }
    };

    // Stay in step with `create_checkout_session`: whatever that arm will not sell through, this one
    // must not claim to have verified.
    if provider != "stripe" {
        return Ok((
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "ok": false,
                "error": "checkout_not_implemented",
                "provider": provider,
                "reason": "checkout_not_implemented",
                "message": format!(
                    "'{provider}' cannot be verified: only Stripe is implemented for checkout in this app, so a key for any other provider would be accepted here and then fail at the buy button."
                ),
            })),
        ));
    }

    let api_base = std::env::var("STRIPE_API_BASE")
        .unwrap_or_else(|_| "https://api.stripe.com".to_string())
        .trim_end_matches('/')
        .to_string();
    let masked = crate::security::provider_key_crypto::mask(&key);

    let sent = reqwest::Client::new()
        .get(format!("{api_base}/v1/balance"))
        .bearer_auth(&key)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await;
    let resp = match sent {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(provider = %provider, key_source = source, error = %e, "payment key test could not reach the provider");
            return Ok((
                StatusCode::FAILED_DEPENDENCY,
                Json(json!({
                    "ok": false,
                    "error": "provider_unreachable",
                    "provider": provider,
                    "key_source": source,
                    "api_key_masked": masked,
                    "reason": "provider_unreachable",
                    "message": format!("Could not reach Stripe at all: {e}"),
                })),
            ));
        }
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if status.is_success() {
        // Stripe's /v1/balance answers `{ "object": "balance", "livemode": <bool>, ... }`. Only
        // `livemode` is read, and reporting it is most of the value: a TEST-mode key is a working key
        // that will never charge anybody, and mistaking one for the other is the exact mistake this
        // route exists to prevent.
        let live = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("livemode").and_then(|b| b.as_bool()));
        let message = match live {
            Some(true) => {
                "Stripe accepted the key and it is a LIVE key — a real customer can be charged."
                    .to_string()
            }
            Some(false) => {
                "Stripe accepted the key but it is a TEST-mode key — no customer will ever be charged. Replace it with the live key before selling.".to_string()
            }
            None => "Stripe accepted the key.".to_string(),
        };
        return Ok((
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "provider": provider,
                "key_source": source,
                "livemode": live,
                "api_key_masked": masked,
                "message": message,
            })),
        ));
    }

    // Never echo the key. Stripe's own sentence is safe and is the whole point of the route.
    let msg = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("Stripe returned {status}"));
    tracing::warn!(provider = %provider, key_source = source, http = %status, "payment key was REFUSED by the provider");
    Ok((
        StatusCode::FAILED_DEPENDENCY,
        Json(json!({
            "ok": false,
            "error": "provider_refused",
            "provider": provider,
            "key_source": source,
            "api_key_masked": masked,
            "reason": "provider_refused",
            "message": format!("Stripe refused the key: {msg}"),
        })),
    ))
}

/// Is this a URL a browser can be sent to in order to pay?
///
/// `plans.purchase_url` is typed in by an operator, so a typo must REFUSE the sale rather than send
/// a buyer somewhere broken or insecure. https only (a payment page over plain http is a downgrade
/// worth refusing), a host with a dot in it, and no whitespace anywhere — enough to catch the two
/// mistakes an operator actually makes (a bare `stripe.com/pay` with no scheme, a pasted line with a
/// trailing space or a newline in the middle) without pretending to be a URL parser.
fn is_hosted_payment_url(raw: &str) -> bool {
    let u = raw.trim();
    let Some(rest) = u.strip_prefix("https://") else {
        return false;
    };
    if u.contains(char::is_whitespace) {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && host.contains('.')
}

/// File the tier a customer pressed "Upgrade" for as an OPEN plan request the operator can act on.
///
/// Returns `Some({"id","plan_slug","plan_name","already_open"})` when a request exists — the row just
/// written, or the one already open for that tier — and `None` when it could not be written.
/// BEST-EFFORT by design: the caller is already answering a 503, so a bookkeeping failure must never
/// turn a refusal into a 500. It is logged instead.
///
/// The row is the SAME shape `plan_request_handler::create` writes for the customer's own "Request
/// the plan" button (`plan_requests`, one open row per (tenant, plan_slug)), because that is what the
/// operator's Plan requests queue lists and what its Resolve arm reads to switch the workspace onto
/// the tier. It is inserted directly rather than through that handler so no receipt mail is queued:
/// this path charged nothing, and a second "we got your request" mail would be noise on top of the
/// screen the customer is already reading.
struct UpgradeAsk<'a> {
    tenant_id: Uuid,
    user_id: Option<Uuid>,
    requested_by: &'a str,
    plan_slug: &'a str,
    plan_name: &'a str,
    plan_price: f64,
    /// The hosted payment page the buyer was handed to, when the plan carries one.
    payment_link: Option<&'a str>,
}

async fn record_upgrade_request(db: &sqlx::PgPool, ask: UpgradeAsk<'_>) -> Option<Value> {
    let UpgradeAsk {
        tenant_id,
        user_id,
        requested_by,
        plan_slug,
        plan_name,
        plan_price,
        payment_link,
    } = ask;
    let open: Option<Uuid> = match sqlx::query_scalar(
        "SELECT id FROM plan_requests \
          WHERE tenant_id = $1 AND plan_slug = $2 AND status IN ('open','contacted') \
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(plan_slug)
    .fetch_optional(db)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("checkout/create: could not look for an open plan request: {e:?}");
            return None;
        }
    };

    if let Some(id) = open {
        return Some(json!({
            "id": id.to_string(),
            "plan_slug": plan_slug,
            "plan_name": plan_name,
            "already_open": true,
        }));
    }

    let written: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO plan_requests (tenant_id, user_id, requested_by, plan_slug, plan_name, note) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(requested_by)
    .bind(plan_slug)
    .bind(plan_name)
    .bind(format!(
        "This workspace pressed Upgrade on the {plan_name} plan (${plan_price:.0}/month) and no \
         payment provider is configured, so nothing was charged and nothing can be billed. Filed \
         automatically by the checkout refusal arm, so the ask survives even if the customer closes \
         the tab. Resolving this request seats the workspace on the plan.{}",
        match payment_link {
            Some(url) => format!(
                " A hosted payment link for this plan ({url}) was opened for the buyer, so CHECK FOR \
                 THE PAYMENT before resolving: a hosted link tells this installation nothing, and \
                 the workspace is only seated when this request is resolved."
            ),
            None => String::new(),
        }
    ))
    .fetch_one(db)
    .await;

    match written {
        Ok(id) => Some(json!({
            "id": id.to_string(),
            "plan_slug": plan_slug,
            "plan_name": plan_name,
            "already_open": false,
        })),
        Err(e) => {
            tracing::error!("checkout/create: could not file the plan request: {e:?}");
            None
        }
    }
}
