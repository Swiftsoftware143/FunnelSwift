//! Affiliate onboarding: "am I an affiliate?", which products can I promote, and choosing them.
//!
//! These three questions drive David's two UI requirements:
//!   * a **gated Affiliate tab** — "gated to anyone who is not an affiliate and open to those who
//!     are" — which cannot exist without a way to ask whether the signed-in user is an affiliate;
//!   * a **product picker** — "so they can pick the product they want to promote" — which needs the
//!     list of promotable products and a way to record a choice.
//!
//! Before this module there was no endpoint for either. `affiliate_selections` existed with the right
//! shape and **zero code references**: nothing could record a choice.
//!
//! The promotable list is built from `affiliate_products` **joined to its tag**
//! (`affiliate_products.system_tag_id -> tags`), because that link is what makes a promotion
//! attributable: `tag_logic::attribute_affiliate_on_tags` credits the affiliate when those tags land on
//! a lead. A product without a tag can be promoted but can never be attributed, so the list reports
//! `tag_name` and flags `has_tag` rather than silently including an unattributable product.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

/// The affiliate row for the signed-in user, if there is one.
async fn my_affiliate(
    state: &AppState,
    auth: &AuthUser,
) -> Result<
    Option<(
        String,
        String,
        Option<f64>,
        Option<f64>,
        bool,
        Option<String>,
    )>,
    AppError,
> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let user_id = Uuid::parse_str(&auth.user_id).ok();

    // Match on the user link first (the signup path sets it), then fall back to email so an affiliate
    // created by an admin — which has no user_id — is still recognised as "already an affiliate"
    // instead of being invited to sign up a second time.
    sqlx::query_as(
        "SELECT id, name, commission_rate::float8, override_commission_rate::float8, is_active, rate_reason \
         FROM affiliates \
         WHERE tenant_id = $1 AND is_active = true AND (user_id = $2 OR email = $3) \
         ORDER BY (user_id = $2) DESC NULLS LAST LIMIT 1",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&auth.email)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from)
}

/// Is the signed-in user an affiliate? Drives both the tab gate and the banner.
pub async fn affiliate_me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let mine = my_affiliate(&state, &auth).await?;
    match mine {
        Some((id, name, rate, override_rate, active, rate_reason)) => {
            // The two signals the rate-band scheduler measures, read here through the SAME definition
            // the recompute uses (see `affiliate_rate_band_handler::apply_rate_bands`) so the dashboard's
            // "next step" and the rate the scheduler will set can never disagree.
            let (paying, apps): (i64, i64) = sqlx::query_as(
                "SELECT count(DISTINCT s.cust),
                        COALESCE(max(s.apps), 0)
                   FROM (
                        SELECT lower(coalesce(nullif(l.email, ''), l.id::text)) AS cust,
                               count(DISTINCT c.metadata->>'source_app') AS apps
                          FROM affiliate_commissions c
                          JOIN leads l ON l.id = c.lead_id
                         WHERE c.affiliate_id = $1
                           AND c.reversed_at IS NULL
                           AND c.status IN ('earned', 'paid')
                         GROUP BY 1
                   ) s",
            )
            .bind(&id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or((0, 0));
            let apps_i = std::cmp::max(apps as i32, 1);
            let effective = override_rate.or(rate);
            // The cheapest active band that pays more than what this affiliate actually earns now.
            let next: Option<(String, f64, i32, i32)> = sqlx::query_as(
                "SELECT label, rate::float8, min_paying_customers, min_apps_per_customer \
                   FROM affiliate_rate_bands \
                  WHERE is_active = true AND rate::float8 > $1 \
                  ORDER BY rate ASC, min_paying_customers ASC LIMIT 1",
            )
            .bind(effective.unwrap_or(0.0))
            .fetch_optional(&state.pool)
            .await?;
            let next_band = next.map(|(label, band_rate, min_cust, min_apps)| {
                json!({
                    "label": label,
                    "rate": band_rate,
                    "min_paying_customers": min_cust,
                    "min_apps_per_customer": min_apps,
                    "paying_customers": paying,
                    "apps_per_customer": apps_i,
                })
            });
            Ok(Json(json!({
                "is_affiliate": true,
                "affiliate": {
                    "id": id, "name": name,
                    "commission_rate": rate,
                    "override_commission_rate": override_rate,
                    "is_active": active,
                    "rate_reason": rate_reason,
                },
                // The rate that actually pays, so the panel can show it next to the tab.
                "effective_rate": effective,
                // Why the standing rate is what it is, and what the next rung requires.
                "next_band": next_band,
            })))
        }
        None => Ok(Json(json!({
            "is_affiliate": false,
            "affiliate": null,
            // The banner offers this only when signing up would not immediately fail.
            "can_sign_up": true,
        }))),
    }
}

/// The products an affiliate may promote, each with its tag and whether this affiliate chose it.
pub async fn promotable_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let mine = my_affiliate(&state, &auth).await?;
    let affiliate_id = mine.as_ref().map(|m| m.0.clone());

    // Each row also carries the tag's PLAN, because a promotion is only meaningful if you can say
    // which plan it lands the customer on: `tags.plan_id` for FunnelSwift's own plans, and
    // `tags.source_app` + `tags.plan_slug` for a sibling app's plan, which lives in a different
    // database and therefore cannot be a foreign key here.
    let rows: Vec<(
        Uuid,
        Option<String>,
        Option<f64>,
        Option<f64>,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        bool,
        Option<String>,
        Option<String>,
        Option<Uuid>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT p.id, p.name, p.price::float8, p.default_commission_rate::float8, \
                p.system_tag_id, t.name, p.source_app, \
                COALESCE(s.is_active, false), \
                COALESCE(t.source_app, p.source_app), t.plan_slug, t.plan_id, \
                COALESCE(pl.name, t.plan_slug) \
         FROM affiliate_products p \
         LEFT JOIN tags t ON t.id = p.system_tag_id \
         LEFT JOIN plans pl ON pl.id = t.plan_id \
         LEFT JOIN affiliate_selections s \
                ON s.product_id = p.id AND s.affiliate_id = $2 \
         WHERE p.is_active IS NOT FALSE \
           AND (p.tenant_id IS NULL OR p.tenant_id = $1 OR p.tenant_id = $3) \
         ORDER BY p.name ASC",
    )
    .bind(tenant_id)
    .bind(&affiliate_id)
    // The fleet catalogue lives in the SYSTEM tenant, not in the caller's: measured 2026-09-29, all
    // five affiliate products belong to 00000000-0000-0000-0000-000000000001 while the signed-in admin
    // is in their own tenant. Filtering on the caller's tenant alone returned an EMPTY catalogue, so
    // the picker had nothing to show and no promotion was possible.
    .bind(crate::system_tenant::system_tenant_id())
    .fetch_all(&state.pool)
    .await?;

    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.0,
                "name": r.1,
                "price": r.2,
                "commission_rate": r.3,
                "tag_id": r.4,
                "tag_name": r.5,
                "source_app": r.6,
                "selected": r.7,
                // ── the tag's FREE PLAN ──────────────────────────────────────────────────────
                // David: "each tag should connect to their free plan". For a sibling app the plan
                // lives in that app's own database, so it is named by (app, slug); for FunnelSwift's
                // own tags it is a real id.
                "plan_app": r.8,
                "plan_slug": r.9,
                "plan_id": r.10,
                "plan_name": r.11,
                "connected_to_plan": r.9.is_some() || r.10.is_some(),
                // A product with no tag can be picked but can never be attributed, so say so
                // instead of letting the affiliate promote something that cannot pay.
                "has_tag": r.4.is_some(),
                "attribution": if r.4.is_some() {
                    format!("credited when the tag '{}' lands on a lead", r.5.clone().unwrap_or_default())
                } else {
                    "NOT attributable: this product has no tag linked yet".to_string()
                },
            })
        })
        .collect();

    Ok(Json(json!({
        "is_affiliate": affiliate_id.is_some(),
        "affiliate_id": affiliate_id,
        "products": items,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct SelectProductsRequest {
    pub product_ids: Vec<Uuid>,
}

/// **Replace** this affiliate's selection — the picker is a set of ticks, so unticking must remove.
pub async fn select_products(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SelectProductsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let Some((affiliate_id, _, _, _, _, _)) = my_affiliate(&state, &auth).await? else {
        // Explicitly not a 403: the caller is authenticated and allowed, they simply are not an
        // affiliate yet — the panel shows the signup CTA for this answer.
        return Err(AppError::Forbidden(
            "Become an affiliate before choosing products to promote".into(),
        ));
    };

    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE affiliate_selections SET is_active = false WHERE affiliate_id = $1")
        .bind(&affiliate_id)
        .execute(&mut *tx)
        .await?;

    let mut selected = 0usize;
    for pid in &req.product_ids {
        // ON CONFLICT needs the partial unique index to be inferable, so the conflict target names
        // the columns and the same predicate as the index.
        sqlx::query(
            "INSERT INTO affiliate_selections (id, tenant_id, product_id, affiliate_id, is_active, code) \
             VALUES ($1, $2, $3, $4, true, $5) \
             ON CONFLICT (affiliate_id, product_id) WHERE affiliate_id IS NOT NULL \
             DO UPDATE SET is_active = true",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(pid)
        .bind(&affiliate_id)
        .bind(format!("{affiliate_id}-{}", &pid.to_string()[..8]))
        .execute(&mut *tx)
        .await?;
        selected += 1;
    }
    tx.commit().await?;

    Ok(Json(json!({
        "affiliate_id": affiliate_id,
        "selected": selected,
        "message": "Your promoted products were updated",
    })))
}

/// Clear one product from this affiliate's selection.
pub async fn deselect_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(product_id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let Some((affiliate_id, _, _, _, _, _)) = my_affiliate(&state, &auth).await? else {
        return Err(AppError::Forbidden(
            "Become an affiliate before changing what you promote".into(),
        ));
    };
    // `AND is_active = true` so a second deselect is genuinely a no-op that reports 0 removed: without
    // it the UPDATE matched the already-inactive row and answered "removed: 1" for work it did not do.
    let res = sqlx::query(
        "UPDATE affiliate_selections SET is_active = false \
         WHERE affiliate_id = $1 AND product_id = $2 AND is_active = true",
    )
    .bind(&affiliate_id)
    .bind(product_id)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::OK,
        Json(json!({
            "removed": res.rows_affected(),
            "message": if res.rows_affected() == 0 {
                "That product was already not in your selection"
            } else {
                "Removed from what you promote"
            }
        })),
    ))
}
