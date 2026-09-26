use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::plan::*;
use crate::state::AppState;
use crate::tag_logic;
use sqlx::Row;

// ── Retired `features` keys ──

/// Refuse a plan `features` object that carries the RETIRED key `card_types`.
///
/// **The decision (kanban t_cbd500ca, 2026-09-26): `plans.features.card_types` is RETIRED**, not
/// promoted to an enforced per-plan card-kind matrix. Every plan that carried it held
/// `["bio-link","digital-card","mini-page"]` — a THIRD spelling that was read by no code at all
/// (`grep -rn "card_types" src/ www-app/ www-admin/ migrations/ docs/ scripts/` found doc comments
/// only) and that cannot be promoted honestly:
///
/// * `digital-card` is not a card type this service can produce (`card_types::canonical` returns
///   `None` for it), while Suite and Agency — which DO carry `"mini_funnels": true` — omit
///   `mini-funnel` from the same list, so promoting it verbatim would both offer a bogus kind and
///   take a real one away from paying plans;
/// * the enforced per-plan card gate already exists and is BOOLEAN: the `has_mini_funnels` column
///   with the `mini_funnels` jsonb override (`features.rs::flag_jsonb_key`, applied by
///   `kinetic_handler.rs::create_card`). The plan matrix's jsonb vocabulary has no list shape at
///   all, and which plan sells which card archetype is a product decision (architecture lane).
///
/// Migration 063 removes the key from every row; this guard is what makes the retirement STICKY —
/// a future UI, seed or gate cannot re-open the drift by writing the key back. The canonical ids are
/// named in the refusal from their ONE declaration, [`crate::card_types::CARD_TYPES`].
fn reject_retired_feature_keys(features: &serde_json::Value) -> AppResult<()> {
    if features.get("card_types").is_some() {
        return Err(AppError::BadRequest(format!(
            "features.card_types is retired (kanban t_cbd500ca) and cannot be written back: it was \
             dead data in a third spelling of the card kinds ([\"bio-link\",\"digital-card\",\
             \"mini-page\"]) read by no code, and `digital-card` is not a card type this service can \
             produce. The card ids are {} (src/card_types.rs); which kinds a plan sells is enforced \
             by the `has_mini_funnels` column and the `mini_funnels` feature flag. Send the features \
             map without `card_types`.",
            crate::card_types::CARD_TYPES.join(", ")
        )));
    }
    Ok(())
}

// ── Affiliate Product Auto-Sync helpers ──

/// THE ONE WRITER of a plan-derived affiliate product (kanban t_6d326447).
///
/// **The decision: ONE WRITER.** Every plan -> product materialisation in this service goes through
/// this function, and this function takes NOT ONE commercial value from its caller: it reads
/// `name`, `price` and `commission_rate` off the `plans` row itself, because the plan is the source
/// of truth for what its product sells at and pays. Callers only decide WHEN a product must exist
/// (`create_plan`/`update_plan` sync eagerly, `admin_sync_affiliate_products` backfills the products
/// that are missing). Neither caller knows a column value any more.
///
/// Measured before the change: the two other materialisation routes each carried their own
/// hand-rolled `INSERT ... VALUES (..., 10.0)` — `POST /api/v1/admin/affiliate-products/sync`
/// (affiliate_product_handler.rs) and `POST /api/v1/internal/sync-affiliate-plan` (the route this
/// card calls `cross-app/plan-sync`) — with a LITERAL 10.0 rate, no description and no category_id.
/// A probe plan with `plans.commission_rate = 7.5` therefore materialised
/// `default_commission_rate = 10.00` through either route and `7.50` through this one: the
/// commission a plan product advertised depended on WHICH route created the row. Both routes now
/// delegate here and carry no value of their own.
///
/// **`is_active` is deliberately NOT synced** (kanban t_9c30ce49). It is a lifecycle flag owned by
/// the product's own screen (`PUT /api/v1/affiliate-products/:id`) and by
/// [`deactivate_affiliate_product_for_plan`] when the plan is deleted. This function rewrites the
/// product's COMMERCIAL fields only: name, description, price, commission. Measured before the
/// change: an admin retired a plan-derived product and then edited the plan's price → the product
/// came back `is_active = true` with no mention in the UI.
///
/// The alternative ("the plan owns it") is not available in this schema: `plans` has no `is_active`
/// column at all, so a plan-level flag would have to be invented (column + migration + an admin
/// control that does not exist) while the product checkbox already writes the real one.
///
/// A brand-new product still starts ACTIVE — it takes the column's own `DEFAULT true`, so a new
/// plan is sellable and this function never mentions the flag.
///
/// Returns the id of the product this plan owns (inserted or refreshed), or `AppError::NotFound`
/// when there is no such plan — a plan-derived product without its plan has no commission to carry.
pub async fn sync_plan_to_affiliate_product(
    pool: &sqlx::PgPool,
    plan_id: uuid::Uuid,
    tenant_id: Option<uuid::Uuid>,
) -> Result<Option<uuid::Uuid>, crate::error::AppError> {
    // The values come from the PLAN, never from a caller (t_6d326447).
    let plan: Option<(String, f64, f64)> = sqlx::query_as(
        "SELECT name, price::float8, commission_rate::float8 FROM plans WHERE id = $1",
    )
    .bind(plan_id)
    .fetch_optional(pool)
    .await?;
    let Some((plan_name, plan_price, commission_rate)) = plan else {
        return Err(crate::error::AppError::NotFound(format!(
            "plan {plan_id} not found — a plan-derived affiliate product takes its name, price and \
             commission from its plan"
        )));
    };
    let name = plan_name.as_str();
    let price = plan_price;

    // Look up the FunnelSwift Plans category; fall back to any available category
    let category_id: Option<uuid::Uuid> = match sqlx::query_scalar(
        "SELECT id FROM product_categories WHERE slug = 'funnelswift-plans' LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    {
        Some(id) => Some(id),
        None => {
            // Fallback: try to get any category
            sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM product_categories LIMIT 1")
                .fetch_optional(pool)
                .await?
        }
    };

    let effective_tenant_id: uuid::Uuid = match tenant_id {
        Some(tid) => tid,
        None => sqlx::query_scalar("SELECT id FROM tenants ORDER BY created_at LIMIT 1")
            .fetch_optional(pool)
            .await?
            .unwrap_or_else(uuid::Uuid::nil),
    };

    // Check if affiliate product already exists for this plan
    let existing: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM affiliate_products WHERE plan_id = $1")
            .bind(plan_id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let description = format!("{} — FunnelSwift Plan", name);

    if existing > 0 {
        // Update existing. `is_active` is deliberately absent from this SET list: a plan edit must
        // not resurrect a product an admin retired (kanban t_9c30ce49).
        sqlx::query(
            r#"UPDATE affiliate_products SET
                name = $1,
                description = $2,
                price = $3,
                default_commission_rate = $4,
                updated_at = NOW()
            WHERE plan_id = $5"#,
        )
        .bind(name)
        .bind(&description)
        .bind(price)
        .bind(commission_rate)
        .bind(plan_id)
        .execute(pool)
        .await?;
    } else {
        // Insert new. `affiliate_products.id` has NO column default, so the id must be supplied or
        // the INSERT dies on a NOT NULL violation (measured: every new plan produced no product at
        // all, because the caller discarded the error — kanban t_9c30ce49). `is_active` is left out
        // on purpose so the column DEFAULT owns "a new plan is sellable".
        sqlx::query(
            r#"INSERT INTO affiliate_products
                (id, tenant_id, name, description, price, default_commission_rate, category_id, plan_id, owner_name, product_type, source_app)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'SwiftSoftware', 'software', 'funnelswift')"#
        )
        .bind(uuid::Uuid::new_v4())
        .bind(effective_tenant_id)
        .bind(name)
        .bind(&description)
        .bind(price)
        .bind(commission_rate)
        .bind(category_id)
        .bind(plan_id)
        .execute(pool)
        .await?;
    }

    // Hand the caller back the product this plan owns, so a route can report what it touched
    // without re-deriving the id (the cross-app sync answers with it).
    let product_id: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM affiliate_products WHERE plan_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(plan_id)
    .fetch_optional(pool)
    .await?;

    Ok(product_id)
}

/// Set the affiliate product to inactive when a plan is deleted, preserving historical data.
async fn deactivate_affiliate_product_for_plan(
    pool: &sqlx::PgPool,
    plan_id: uuid::Uuid,
) -> Result<(), crate::error::AppError> {
    sqlx::query(
        "UPDATE affiliate_products SET is_active = false, updated_at = NOW() WHERE plan_id = $1 AND is_active = true"
    )
    .bind(plan_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// The `plans` column list and the `Plan` row shape, in ONE place.
///
/// `SELECT *` broke at runtime because `commission_rate` is NUMERIC in the live DB while
/// `Plan.commission_rate` is `Option<f64>` — sqlx fails the whole query with
/// "Rust Option<f64> (FLOAT8) not compatible with SQL type NUMERIC", so `GET /api/v1/plans`
/// returned HTTP 500. Casting the numeric columns to float8 keeps the struct unchanged.
///
/// This is a MACRO, not a `const` spliced in with `format!`: `concat!` takes literals, so the
/// query text is assembled at COMPILE time and no SQL is built at run time (pre-build gate rule
/// 5d, kanban t_92ce05b3). Values were always bound; only the column list is interpolated.
macro_rules! plan_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, name, slug, price::float8 AS price, \
    annual_price::float8 AS annual_price, commission_rate::float8 AS commission_rate, \
    side, max_custom_domains, max_cards, max_qr_codes, max_action_buttons, max_forms, \
    max_leads, max_tags, max_team_members, max_ocr_scans, has_webhooks, has_api, \
    has_dual_routing, has_mini_funnels, has_card_gating, has_remove_branding, \
    has_white_label, has_multi_tenant, has_analytics, has_import_export, billing_cycle, \
    purchase_url, payment_provider, features, created_at, updated_at FROM plans ",
            $tail
        )
    };
}

pub async fn list_plans(State(state): State<AppState>) -> AppResult<Json<Vec<Plan>>> {
    let plans = sqlx::query_as::<_, Plan>(plan_select!("ORDER BY price"))
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(plans))
}

pub async fn create_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePlanRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    if let Some(f) = &req.features {
        reject_retired_feature_keys(f)?;
    }
    let plan_id = Uuid::new_v4();
    // Column list == placeholder list == bind list, or this INSERT dies before it runs. It was
    // broken twice over (kanban t_9c30ce49, both measured live: `POST /api/v1/plans` answered
    // 500 "Database error" for every caller):
    //   1. 29 columns with 30 placeholders -> `INSERT has more expressions than target columns`;
    //   2. `max_custom_domains` and `max_team_members` are NOT NULL columns with their own DEFAULT,
    //      and binding `Option<i32> = None` sent an explicit NULL -> `null value in column
    //      "max_custom_domains" violates not-null constraint`.
    // Left out here so the COLUMN DEFAULTS own them (0 and 2); the nullable limits below still
    // accept NULL as "no limit recorded".
    sqlx::query(
        r#"INSERT INTO plans (id, name, slug, price, annual_price, side, billing_cycle, max_cards, max_qr_codes, max_action_buttons, max_forms, max_leads, max_tags, max_ocr_scans, has_webhooks, has_api, has_dual_routing, has_mini_funnels, has_card_gating, has_remove_branding, has_white_label, has_multi_tenant, has_analytics, has_import_export, purchase_url, payment_provider, features)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27)"#,
    )
    .bind(plan_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(req.price)
    .bind(req.annual_price)
    .bind(req.side.as_deref().unwrap_or("main"))
    .bind(req.billing_cycle.as_deref().unwrap_or("month"))
    .bind(req.max_cards)
    .bind(req.max_qr_codes)
    .bind(req.max_action_buttons)
    .bind(req.max_forms)
    .bind(req.max_leads)
    .bind(req.max_tags)
    .bind(req.max_ocr_scans)
    .bind(req.has_webhooks.unwrap_or(false))
    .bind(req.has_api.unwrap_or(false))
    .bind(req.has_dual_routing.unwrap_or(false))
    .bind(req.has_mini_funnels.unwrap_or(false))
    .bind(req.has_card_gating.unwrap_or(false))
    .bind(req.has_remove_branding.unwrap_or(false))
    .bind(req.has_white_label.unwrap_or(false))
    .bind(req.has_multi_tenant.unwrap_or(false))
    .bind(req.has_analytics.unwrap_or(false))
    .bind(req.has_import_export.unwrap_or(false))
    .bind(&req.purchase_url)
    .bind(&req.payment_provider)
        .bind(&req.features)
        .execute(&state.pool)
    .await?;

    if let Some(rate) = req.commission_rate {
        sqlx::query("UPDATE plans SET commission_rate = $1 WHERE id = $2")
            .bind(rate)
            .bind(plan_id)
            .execute(&state.pool)
            .await?;
    }

    // Auto-sync to affiliate products. Non-fatal, but never silent: this used to be `let _ =`,
    // which is how the sync's INSERT failure stayed invisible for every new plan (t_9c30ce49).
    // ONE WRITER (t_6d326447): no values are passed — the helper reads them off the plan row it
    // just wrote, so this call site cannot invent a name, a price or a commission.
    if let Err(e) = sync_plan_to_affiliate_product(&state.pool, plan_id, None).await {
        tracing::warn!(plan_id = %plan_id, error = %e, "affiliate product sync failed");
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": plan_id, "message": "Plan created"})),
    ))
}

pub async fn get_plan(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Plan>> {
    let plan = sqlx::query_as::<_, Plan>(plan_select!("WHERE id = $1"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    Ok(Json(plan))
}

pub async fn update_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePlanRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    if let Some(f) = &req.features {
        reject_retired_feature_keys(f)?;
    }
    let existing = sqlx::query_as::<_, Plan>(plan_select!("WHERE id = $1"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    let sync_name = req.name.clone().unwrap_or_else(|| existing.name.clone());
    let sync_price = req.price.unwrap_or(existing.price);
    let sync_slug = req.slug.clone().unwrap_or_else(|| existing.slug.clone());

    // A plan edit that does not mention the rate must KEEP the plan's rate. It used to fall back to
    // a literal 20.0, so any admin save that left the field out silently rewrote the plan's own
    // commission (a 50% plan became 20%) — the same "a caller invents a default instead of reading
    // the plan" defect as the two literal-10.0 product INSERTs this card removes (t_6d326447).
    let sync_rate = req
        .commission_rate
        .or(existing.commission_rate)
        .unwrap_or(20.0);
    sqlx::query(
        r#"UPDATE plans SET name=$1, slug=$2, price=$3, purchase_url=$4, max_leads=$5, max_tags=$6,
           has_dual_routing=$7, has_multi_tenant=$8, has_white_label=$9, payment_provider=$10, features=$11, commission_rate=$12, updated_at=NOW()
           WHERE id=$13"#,
    )
    .bind(&sync_name)
    .bind(&sync_slug)
    .bind(sync_price)
    .bind(&req.purchase_url)
    .bind(req.max_leads.or(existing.max_leads))
    .bind(req.max_tags.or(existing.max_tags))
    .bind(req.has_dual_routing.unwrap_or(existing.has_dual_routing))
    .bind(req.has_multi_tenant.unwrap_or(existing.has_multi_tenant))
    .bind(req.has_white_label.unwrap_or(existing.has_white_label))
    .bind(req.payment_provider.or(existing.payment_provider))
    .bind(req.features.or(existing.features))
    .bind(sync_rate)
    .bind(id)
    .execute(&state.pool)
    .await?;

    // Auto-sync to affiliate products. NOTE: this rewrites the product's name/description/price/
    // commission only — never `is_active`, so editing a plan cannot resurrect a product an admin
    // retired (kanban t_9c30ce49). Non-fatal, but logged: it used to be discarded silently.
    // ONE WRITER (t_6d326447): no values are passed — the helper reads the plan row this UPDATE
    // just wrote, so the product can only ever follow the plan this route persisted.
    if let Err(e) = sync_plan_to_affiliate_product(&state.pool, id, None).await {
        tracing::warn!(plan_id = %id, error = %e, "affiliate product sync failed");
    }

    Ok(Json(json!({"message": "Plan updated"})))
}

pub async fn delete_plan_admin(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    // The plan has to exist BEFORE anything is retired: `affiliate_products.plan_id` is
    // ON DELETE SET NULL, so the join key is gone the instant the plan row goes.
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plans WHERE id = $1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound("Plan not found".into()));
    }

    // Deactivate the affiliate product FIRST (don't delete it — preserve historical conversion
    // data). Measured with the old delete-then-deactivate order: plan_id was already NULL by the
    // time the UPDATE ran, it matched 0 rows, and deleting a plan left its product `is_active =
    // true` (kanban t_9c30ce49). The error is propagated, not discarded: a retirement that did not
    // happen must not answer 200.
    deactivate_affiliate_product_for_plan(&state.pool, id).await?;

    let result = sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Plan not found".into()));
    }

    Ok(Json(json!({"message": "Plan deleted"})))
}

// ── Admin endpoints (follow funnelswift pattern - no auth extractor) ──

pub async fn admin_list_all_plans(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plans = sqlx::query("SELECT id, name, slug, price, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label, payment_provider, features, allowed_template_ids, created_at, updated_at FROM plans ORDER BY price")
        .fetch_all(&state.pool)
        .await?;

    let result: Vec<serde_json::Value> = plans.iter().map(|row| {
        json!({
            "id": row.try_get::<Uuid, _>("id").map(|u| u.to_string()).unwrap_or_default(),
            "name": row.try_get::<String, _>("name").unwrap_or_default(),
            "slug": row.try_get::<String, _>("slug").unwrap_or_default(),
            "price": row.try_get::<f64, _>("price").unwrap_or(0.0),
            "max_leads": row.try_get::<Option<i32>, _>("max_leads").unwrap_or(None),
            "max_tags": row.try_get::<Option<i32>, _>("max_tags").unwrap_or(None),
            "payment_provider": row.try_get::<Option<String>, _>("payment_provider").unwrap_or(None),
            "has_dual_routing": row.try_get::<bool, _>("has_dual_routing").unwrap_or(false),
            "has_multi_tenant": row.try_get::<bool, _>("has_multi_tenant").unwrap_or(false),
            "has_white_label": row.try_get::<bool, _>("has_white_label").unwrap_or(false),
            "features": row.try_get::<Option<serde_json::Value>, _>("features").unwrap_or(None),
            "allowed_template_ids": row.try_get::<Option<Vec<String>>, _>("allowed_template_ids").unwrap_or(None),
        })
    }).collect();

    Ok(Json(json!({"plans": result, "total": result.len()})))
}

pub async fn admin_create_plan_json(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plan_id = Uuid::new_v4();
    let name = req
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let slug = req
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or(&name.to_lowercase().replace(" ", "-"))
        .to_string();
    let price = req.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let max_leads = req
        .get("max_leads")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let max_tags = req
        .get("max_tags")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let has_dual_routing = req
        .get("has_dual_routing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_multi_tenant = req
        .get("has_multi_tenant")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_white_label = req
        .get("has_white_label")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let purchase_url = req
        .get("purchase_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let payment_provider = req
        .get("payment_provider")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let features = req.get("features").cloned();
    if let Some(f) = &features {
        reject_retired_feature_keys(f)?;
    }
    if name.is_empty() {
        return Err(AppError::BadRequest("Plan name is required".into()));
    }

    sqlx::query(
        r#"INSERT INTO plans (id, name, slug, price, purchase_url, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label, payment_provider, features)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(plan_id)
    .bind(&name)
    .bind(&slug)
    .bind(price)
    .bind(&purchase_url)
    .bind(max_leads)
    .bind(max_tags)
    .bind(has_dual_routing)
    .bind(has_multi_tenant)
    .bind(has_white_label)
    .bind(&payment_provider)
    .bind(&features)
    .execute(&state.pool)
    .await?;

    if let Some(rate) = req.get("commission_rate").and_then(|v| v.as_f64()) {
        sqlx::query("UPDATE plans SET commission_rate = $1 WHERE id = $2")
            .bind(rate)
            .bind(plan_id)
            .execute(&state.pool)
            .await?;
    }

    // Auto-sync to affiliate products (commercial fields only — never `is_active`, t_9c30ce49).
    // ONE WRITER (t_6d326447): the values come from the plan row, never from this payload.
    if let Err(e) = sync_plan_to_affiliate_product(&state.pool, plan_id, None).await {
        tracing::warn!(plan_id = %plan_id, error = %e, "affiliate product sync failed");
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": plan_id, "message": "Plan created"})),
    ))
}

pub async fn admin_update_plan_features(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let features = req
        .get("features")
        .ok_or_else(|| AppError::BadRequest("features object is required".to_string()))?;
    reject_retired_feature_keys(features)?;
    let features_str = features.to_string();

    sqlx::query(
        "UPDATE plans SET features = features::jsonb || $1::jsonb, updated_at = NOW() WHERE id = $2",
    )
    .bind(&features_str)
    .bind(id)
    .execute(&state.pool)
    .await?;

    // Also update allowed_template_ids if provided
    if let Some(templates) = req.get("allowed_template_ids") {
        if let Some(arr) = templates.as_array() {
            let ids: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            sqlx::query("UPDATE plans SET allowed_template_ids = $1 WHERE id = $2")
                .bind(&ids)
                .bind(id)
                .execute(&state.pool)
                .await?;
        } else if templates.is_null() {
            sqlx::query("UPDATE plans SET allowed_template_ids = NULL WHERE id = $1")
                .bind(id)
                .execute(&state.pool)
                .await?;
        }
    }

    Ok(Json(json!({"message": "Features updated"})))
}

/// Activate `plan_id` for `tenant_id` and return `(subscription_id, plan_slug)`.
///
/// This is the **only** writer of a tenant's plan. `tenants.plan_id` has never existed in this
/// database (`ERROR 42703`), and the plan really is the active `tenant_plan_subscriptions` row:
/// that is what `list_tenants` reads (LATERAL … WHERE status = 'active') and what this module's
/// own `admin_assign_plan` writes. Returns the slug so callers can run slug-dependent side
/// effects (sold-tag routing).
pub(crate) async fn set_active_plan(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    plan_id: Uuid,
) -> AppResult<(Uuid, String)> {
    // Get the new plan slug before activating
    let new_plan_slug: String = sqlx::query_scalar("SELECT slug FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    // Deactivate existing subscription first
    sqlx::query(
        "UPDATE tenant_plan_subscriptions SET status = 'cancelled' WHERE tenant_id = $1 AND status = 'active'"
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date)
           VALUES ($1, $2, $3, 'active', NOW())"#,
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(plan_id)
    .execute(pool)
    .await?;

    Ok((subscription_id, new_plan_slug))
}

pub async fn admin_assign_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id = req
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("Valid tenant_id is required".into()))?;
    let plan_id = req
        .get("plan_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("Valid plan_id is required".into()))?;

    // Get the new plan slug before activating
    let (subscription_id, new_plan_slug) = set_active_plan(&state.pool, tenant_id, plan_id).await?;

    // Auto-apply Sold tag to all leads in this tenant
    // This only applies when upgrading to paid plans (pro/enterprise)
    tag_logic::apply_sold_to_tenant_leads(&state.pool, tenant_id, &new_plan_slug).await?;

    Ok(Json(
        json!({"message": "Plan assigned to tenant", "subscription_id": subscription_id}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `features.card_types` blurb every plan carried, kept as the NEGATIVE case of the
    /// card-type value space (kanban t_cbd500ca: the key is RETIRED, migration 063).
    const RETIRED_PLAN_CARD_TYPES: &[&str] = &["bio-link", "digital-card", "mini-page"];

    /// Not one element of the retired blurb is a canonical card id, which is what "the value space
    /// stays single" means for this key: `bio-link`/`mini-page` are ALIASES that the card writes
    /// normalise (`canonical()` maps them to `bio_link`/`mini_page`), so a gate comparing the stored
    /// blurb verbatim against `CARD_TYPES` would match neither of them, and `digital-card` maps to
    /// nothing at all (this service cannot produce it).
    #[test]
    fn no_element_of_the_retired_blurb_is_a_canonical_card_id() {
        for raw in RETIRED_PLAN_CARD_TYPES {
            assert!(
                !crate::card_types::CARD_TYPES.contains(raw),
                "{raw} is a canonical card id after all - the retirement would then be wrong"
            );
            assert!(
                crate::card_types::canonical(raw) != Some(raw),
                "{raw} normalises to itself - it would be storable as a card type"
            );
        }
        assert_eq!(crate::card_types::canonical("digital-card"), None);
    }

    #[test]
    fn the_retired_key_is_refused_wherever_it_appears() {
        // the shape the four live rows had
        let err = reject_retired_feature_keys(&json!({
            "card_types": ["bio-link", "digital-card", "mini-page"],
            "premium_themes": true
        }))
        .expect_err("a features map carrying card_types must be refused");
        let msg = match err {
            AppError::BadRequest(m) => m,
            other => panic!("expected 400 BadRequest, got {other:?}"),
        };
        // the refusal names the canonical ids from their ONE declaration, not from prose
        for id in crate::card_types::CARD_TYPES {
            assert!(msg.contains(id), "refusal does not name {id}: {msg}");
        }
        assert!(msg.contains("card_types") && msg.contains("has_mini_funnels"));

        // the key counts even when it is null - it must not exist at all
        assert!(reject_retired_feature_keys(&json!({"card_types": null})).is_err());
        assert!(reject_retired_feature_keys(&json!({"card_types": []})).is_err());
    }

    #[test]
    fn every_other_features_map_and_shape_still_sails_through() {
        assert!(reject_retired_feature_keys(&json!({
            "description": "A free professional digital business card for networking",
            "custom_domain": false,
            "video_background": false,
            "premium_themes": true,
            "mini_funnels": true
        }))
        .is_ok());
        // a features value that is not an object has no such key; the guard owns only the key
        assert!(reject_retired_feature_keys(&json!(null)).is_ok());
        assert!(reject_retired_feature_keys(&json!({})).is_ok());
    }
}
