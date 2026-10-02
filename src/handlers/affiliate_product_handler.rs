// Affiliate product handler - full CRUD with admin variants
// David's model: affiliate products = the SwiftSoftware products (CoreSwift, FunnelSwift,
// IncentiveSwift, MultiDirectory, etc.). Admin adds products and assigns a system
// tag to each. When a lead gets that system tag, the affiliate system records attribution.
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `is_active` is deliberately `Option<Value>` and not `Option<bool>`: the flag is the ONE field
/// this card is about, and a value that is neither `true` nor `false` must be REFUSED with the app's
/// own 400 JSON body (`parse_is_active`) rather than swallowed by a serde default — that is the same
/// arm `tenant_handler::parse_status` gives `tenants.status`. Typing it as `bool` would also answer
/// a bad value with axum's opaque 422 instead of `{"error":…,"message":…}`.
#[derive(Debug, Deserialize)]
pub struct CreateProductRequest {
    pub name: String,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub category_id: Option<Uuid>,
    pub url: Option<String>,
    pub is_third_party: Option<bool>,
    pub product_type: Option<String>,
    pub owner_name: Option<String>,
    pub system_tag_id: Option<Uuid>,
    pub is_active: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub is_active: Option<Value>,
    pub category_id: Option<Uuid>,
    pub url: Option<String>,
    pub is_third_party: Option<bool>,
    pub product_type: Option<String>,
    pub owner_name: Option<String>,
    pub system_tag_id: Option<Uuid>,
    /// THE REVERSAL (kanban t_2b82d8f0). `system_tag_id` cannot express "remove the tag": a JSON
    /// `null` decodes to `None`, and `None` means "keep the stored value", so before this field the
    /// link the affiliate-attribution reader keys on `tag_logic::attribute_affiliate_on_tags`
    /// (src/tag_logic.rs:319) was ONE-WAY — an admin could set it and never unset it (measured live
    /// on c4311cc1: PUT `{"system_tag_id": null}` left the column untouched, so the product form could
    /// only ever add a tag). Additive on purpose: with this absent/false every existing caller keeps
    /// today's behaviour byte for byte; `true` with no `system_tag_id` clears the column.
    pub clear_system_tag: Option<bool>,
}

/// Optional list filter on the product's own lifecycle flag (kanban t_db6fa3c0).
///
/// ABSENT means "every row", and that default is the decision, not an oversight: this list backs the
/// admin screen that owns the checkbox (`LP`, www-app/index.html), so filtering it would hide an
/// inactive product from the only place that can re-tick it — the switch would be one-way. The
/// affiliate-facing catalog (`LPR`) is the caller that asks for `is_active=true`, because a retired
/// product stops being the thing that gets attributed: both attribution readers resolve a product
/// `WHERE is_active = true` only — `tag_logic::attribute_affiliate_on_tags` (src/tag_logic.rs:319,
/// which then writes no commission at all for it) and
/// `affiliate_tracking_handler::handle_affiliate_upgrade_event`
/// (src/handlers/affiliate_tracking_handler.rs:207, which binds the resolved id into the
/// commission's `product_id`, so an inactive product leaves it NULL).
#[derive(Debug, Deserialize)]
pub struct ProductListQuery {
    pub is_active: Option<bool>,
}

/// The ONE reader of the `is_active` request value. `None`, or an explicit JSON `null`, means "the
/// caller did not send it" and the fallback (the stored value on update, `true` on create) is used,
/// so a save that does not touch the checkbox cannot flip the flag. Anything else must be a JSON
/// boolean; a string/`1`/object is a 400 instead of a silent no-op.
fn parse_is_active(raw: Option<&Value>, fallback: bool) -> AppResult<bool> {
    match raw {
        Some(v) if !v.is_null() => v.as_bool().ok_or_else(|| {
            AppError::BadRequest(format!("Invalid is_active '{v}': expected true or false"))
        }),
        _ => Ok(fallback),
    }
}

/// The Affiliate Product screen's Category select is built from
/// `product_category_handler::list_categories`, whose predicate is "this tenant's categories OR the
/// fleet's system-tenant taxonomy" (the system rows are what `plan_handler` links the plan-derived
/// products to). A `category_id` a caller supplies must come from that same row set, so resolve it
/// with the same predicate BEFORE the writer runs.
///
/// Why the app has to check at all (measured live, kanban t_45d9684d): the FK added by migration 064
/// (`affiliate_products_category_id_fkey`) is a plain reference to `product_categories(id)`, and that
/// id is a global PK — so the edge refuses an id that references NOTHING (a 500 before this change)
/// but happily accepts another REAL workspace's category, which the select never offers. Both are the
/// same caller mistake and both answer the same field-level 400, deliberately with ONE message so the
/// response cannot be used to probe which category ids exist in other workspaces.
async fn ensure_category_offered(
    state: &AppState,
    tenant_id: Uuid,
    category_id: Uuid,
) -> AppResult<()> {
    let offered: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM product_categories WHERE id = $1 AND (tenant_id = $2 OR tenant_id = $3)",
    )
    .bind(category_id)
    .bind(tenant_id)
    .bind(crate::system_tenant::system_tenant_id())
    .fetch_optional(&state.pool)
    .await?;
    if offered.is_none() {
        return Err(AppError::BadRequest("Unknown category".into()));
    }
    Ok(())
}

/// A category can still disappear between the check above and the write (another admin deleting it —
/// exactly the stale-tab case this card is about). `23503` on
/// `affiliate_products_category_id_fkey` is that race arriving at the database; it gets the SAME
/// field-level 400 instead of the generic "Database error" 500. `constraint()` is exact, so the
/// table's other FKs (plan_id, system_tag_id, tenant_id) keep their generic handling.
fn category_fk_error(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db) = &e {
        if is_category_fk(db.code().as_deref(), db.constraint()) {
            return AppError::BadRequest("Unknown category".into());
        }
    }
    AppError::Database(e)
}

/// The decision `category_fk_error` makes, split out so it can be asserted without faking a driver
/// error: ONLY SQLSTATE 23503 (foreign_key_violation) on ONLY this constraint name.
fn is_category_fk(code: Option<&str>, constraint: Option<&str>) -> bool {
    code == Some("23503") && constraint == Some("affiliate_products_category_id_fkey")
}

#[derive(Debug, sqlx::FromRow)]
struct AffiliateProductRow {
    pub id: Uuid,
    // NULLABLE-DECODED-AS-NON-OPTION (struct), kanban t_d5da34d0. `tenant_id` and `name` have no
    // DEFAULT (a NULL is real data) so they decode as Option; `is_active` carries DEFAULT true and
    // keeps its Rust type through COALESCE in the SELECT. `created_at`/`updated_at` are the same
    // class one layer down: their NULLABILITY is invisible to `fleet-dbtype-audit.py` because the
    // items are casts (`ap.created_at::timestamp`), and every INSERT in this app omits both
    // columns, so a NULL is unreachable - but a psql write of NULL used to fail the whole-row
    // decode of the admin product list, so they are Option too.
    pub tenant_id: Option<Uuid>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub is_active: bool,
    pub is_third_party: Option<bool>,
    pub url: Option<String>,
    pub category_id: Option<Uuid>,
    pub product_type: Option<String>,
    pub owner_name: Option<String>,
    pub system_tag_id: Option<Uuid>,
    pub system_tag_name: Option<String>,
    /// CATEGORY-NAME-FOR-THE-SCREEN (kanban t_a0214025). Both served product screens render the
    /// product's category as a NAME (`p.category_name` in www-app/index.html, `LP` and the
    /// affiliate-facing `LPR`), but this response carried only `category_id`, so the admin list's
    /// Category cell read "-" for every row whatever it held — the same silent-drop class as
    /// `is_active` in t_db6fa3c0. The name arrives through the same LEFT JOIN shape
    /// `system_tag_name` already uses for `tags`, and it stays a LEFT JOIN (not an inner one):
    /// since migration 064 the column carries `affiliate_products_category_id_fkey`
    /// (`ON DELETE SET NULL`), so a dangling id can no longer be produced by the app — the LEFT JOIN
    /// is kept as defence, so that even an out-of-band orphan still lists its row with
    /// `category_name: null` rather than dropping the product out of the list.
    pub category_name: Option<String>,
    pub created_at: Option<chrono::NaiveDateTime>,
    pub updated_at: Option<chrono::NaiveDateTime>,
    /// THE FREE PLAN THIS PRODUCT LANDS A REFERRED CUSTOMER ON.
    ///
    /// David's rule: *"everyone upgrades in app"* — so a referred customer must arrive on the FREE
    /// plan of that software and pay for the upgrade themselves. The column has always been in the
    /// table and the admin guide has always told admins that each row shows it, but the API never
    /// returned it, so neither the affiliate's picker nor the admin screen could show or audit which
    /// plan a product points at. `migration 070`'s trigger
    /// (`enforce_affiliate_products_are_free`) is what refuses a paid plan; this is what lets a
    /// human SEE what was refused or accepted.
    pub plan_id: Option<Uuid>,
    pub plan_slug: Option<String>,
    pub plan_name: Option<String>,
    pub plan_price: Option<f64>,
}

pub async fn list_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<ProductListQuery>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let products: Vec<AffiliateProductRow> = sqlx::query_as(
        // $2 IS NULL = "no filter" (every row, the admin screen's list); a value filters on the
        // product's own lifecycle flag. COALESCE because the column is NULLABLE with DEFAULT true.
        "SELECT ap.id, ap.tenant_id, ap.name, ap.description, ap.price::float8, ap.default_commission_rate::float8,
                COALESCE(ap.is_active, true) AS is_active, ap.is_third_party, ap.url, ap.category_id,
                ap.product_type, ap.owner_name, ap.system_tag_id,
                t.name AS system_tag_name, pc.name AS category_name,
                ap.created_at::timestamp, ap.updated_at::timestamp,
                ap.plan_id, pl.slug AS plan_slug, pl.name AS plan_name,
                COALESCE(pl.price, 0)::float8 AS plan_price
         FROM affiliate_products ap
         LEFT JOIN tags t ON t.id = ap.system_tag_id
         LEFT JOIN product_categories pc ON pc.id = ap.category_id
         LEFT JOIN plans pl ON pl.id = ap.plan_id
         WHERE (ap.tenant_id = $1 OR ap.tenant_id = $3)
           AND ($2::boolean IS NULL OR COALESCE(ap.is_active, true) = $2)
         ORDER BY ap.created_at DESC",
    )
    .bind(tenant_id)
    .bind(params.is_active)
    // $3 — the fleet's system tenant: the plan-derived products it owns are part of every
    // admin's list. BOUND, never a UUID literal in the SQL text (kanban t_92ce05b3).
    .bind(crate::system_tenant::system_tenant_id())
    .fetch_all(&state.pool)
    .await?;

    let result: Vec<Value> = products
        .iter()
        .map(|p| {
            json!({
                "id": p.id.to_string(),
                "name": p.name,
                "description": p.description,
                "price": p.price.unwrap_or(0.0),
                "default_commission_rate": p.default_commission_rate.unwrap_or(0.0),
                "is_active": p.is_active,
                "is_third_party": p.is_third_party.unwrap_or(false),
                "url": p.url,
                "category_id": p.category_id.map(|v| v.to_string()),
                // The NAME the screen renders next to the id (kanban t_a0214025). Null when the
                // product has no category — or when the stored id points at a row that no longer
                // exists, which migration 064 (FK + ON DELETE SET NULL) makes unreachable through
                // the app; the screen then shows its own "-" placeholder either way.
                "category_name": p.category_name,
                "product_type": p.product_type.as_deref().unwrap_or("software"),
                "owner_name": p.owner_name.as_deref().unwrap_or("SwiftSoftware"),
                "system_tag_id": p.system_tag_id.map(|v| v.to_string()),
                "system_tag_name": p.system_tag_name,
                "created_at": p.created_at,
                "updated_at": p.updated_at,
                // The free plan a referred customer lands on, so both screens can show it.
                "plan_id": p.plan_id.map(|v| v.to_string()),
                "plan_slug": p.plan_slug,
                "plan_name": p.plan_name,
                "plan_price": p.plan_price.unwrap_or(0.0),
                // True when the product has no plan to state, which is legitimate for a product
                // owned by ANOTHER app (its free plan lives in that app's database) and worth
                // flagging rather than hiding: an unstated plan is exactly where a paid giveaway
                // could hide if it were ever misused.
                "plan_stated": p.plan_id.is_some(),
            })
        })
        .collect();

    Ok(Json(json!(result)))
}

pub async fn list_all_affiliate_products_admin(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<ProductListQuery>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    list_affiliate_products(auth, State(state), Query(params)).await
}

/// List system tags available for assignment to affiliate products (admin dropdown).
pub async fn list_system_tags(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tags: Vec<(Uuid, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, color FROM tags WHERE is_system = true ORDER BY name")
            .fetch_all(&state.pool)
            .await?;
    let result: Vec<Value> = tags
        .iter()
        .map(|(id, name, color)| json!({ "id": id.to_string(), "name": name, "color": color }))
        .collect();
    Ok(Json(json!(result)))
}

pub async fn create_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateProductRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let id = Uuid::new_v4();
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Absent / null keeps the column default (true) exactly as before this change.
    let is_active = parse_is_active(req.is_active.as_ref(), true)?;
    // Only a caller-SUPPLIED pointer is checked, and against the select's own row set.
    if let Some(category_id) = req.category_id {
        ensure_category_offered(&state, tenant_id, category_id).await?;
    }

    sqlx::query(
        "INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, url, category_id, product_type, owner_name, system_tag_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price.unwrap_or(0.0))
    .bind(req.default_commission_rate.unwrap_or(0.0))
    .bind(is_active)
    .bind(req.is_third_party.unwrap_or(false))
    .bind(&req.url)
    .bind(req.category_id)
    .bind(req.product_type.unwrap_or_else(|| "software".to_string()))
    .bind(req.owner_name.unwrap_or_else(|| "SwiftSoftware".to_string()))
    .bind(req.system_tag_id)
    .execute(&state.pool)
    .await
    .map_err(category_fk_error)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.to_string(), "message": "Product created"})),
    ))
}

pub async fn update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProductRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // affiliate_products.name is NULLABLE with no default: decoding it as String
    // 500'd the whole update on any row that had none. Option + `.or()` keeps the
    // stored value (NULL included) when the request does not supply one.
    // `is_active` is read back too (COALESCE, because the column is NULLABLE with DEFAULT true) so a
    // save that does not carry the key keeps the stored flag instead of re-ticking the row.
    let existing = sqlx::query_as::<_, (Option<String>, Option<String>, f64, f64, bool, Option<String>, Option<Uuid>, Option<String>, Option<String>, Option<Uuid>, bool)>(
        // COALESCE(numeric, 0.0) is still NUMERIC and sqlx refuses to decode NUMERIC
        // into f64 ("mismatched types; Rust type `f64` is not compatible with SQL type
        // `NUMERIC`"), so this route 500'd for EVERY row once the decode was reached.
        // Cast to float8 like the list path above (ap.price::float8, line 73).
        "SELECT name, description, COALESCE(price,0.0)::float8, COALESCE(default_commission_rate,0.0)::float8, COALESCE(is_third_party,false), url, category_id, product_type, owner_name, system_tag_id, COALESCE(is_active,true)
         FROM affiliate_products WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Product not found".into()))?;

    let name = req.name.or(existing.0);
    let description = req.description.or(existing.1);
    let price = req.price.unwrap_or(existing.2);
    let commission = req.default_commission_rate.unwrap_or(existing.3);
    let is_third_party = req.is_third_party.unwrap_or(existing.4);
    let url = req.url.or(existing.5);
    // Only a caller-SUPPLIED pointer is checked (an absent key or an explicit null keeps the stored
    // value, which migration 064 already detaches when its category is deleted), and it is checked
    // against the same row set the screen's select is built from.
    if let Some(category_id) = req.category_id {
        ensure_category_offered(&state, tenant_id, category_id).await?;
    }
    let category_id = req.category_id.or(existing.6);
    let product_type = req
        .product_type
        .unwrap_or(existing.7.unwrap_or_else(|| "software".to_string()));
    let owner_name = req
        .owner_name
        .unwrap_or(existing.8.unwrap_or_else(|| "SwiftSoftware".to_string()));
    // system_tag_id: an absent key or an explicit null KEEPS the stored tag (measured on the deployed
    // binary, kanban t_2b82d8f0: a PUT carrying `system_tag_id: null` left the column untouched, so the
    // comment that used to sit here — "explicit Some(null) clears the tag" — described a behaviour the
    // route never had). Clearing therefore has its own additive flag, `clear_system_tag`; an explicit id
    // still wins if a caller somehow sends both.
    let system_tag_id = match (req.clear_system_tag.unwrap_or(false), req.system_tag_id) {
        (true, Some(v)) => Some(v),
        (true, None) => None,
        (false, v) => v.or(existing.9),
    };
    // Absent key / null = keep the stored flag (a no-touch save must not re-tick a retired product);
    // an explicit true/false persists; anything else is a 400 (see parse_is_active).
    let is_active = parse_is_active(req.is_active.as_ref(), existing.10)?;

    sqlx::query(
        "UPDATE affiliate_products SET name=$1, description=$2, price=$3, default_commission_rate=$4,
         is_third_party=$5, url=$6, category_id=$7, product_type=$8, owner_name=$9, system_tag_id=$10,
         is_active=$11, updated_at=NOW()
         WHERE id=$12 AND tenant_id=$13"
    )
    .bind(&name)
    .bind(&description)
    .bind(price)
    .bind(commission)
    .bind(is_third_party)
    .bind(&url)
    .bind(category_id)
    .bind(&product_type)
    .bind(&owner_name)
    .bind(system_tag_id)
    .bind(is_active)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await
    .map_err(category_fk_error)?;

    Ok(Json(json!({"message": "Product updated"})))
}

pub async fn delete_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let result = sqlx::query("DELETE FROM affiliate_products WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Product not found".into()));
    }

    Ok(Json(json!({"message": "Product deleted"})))
}

/// BACKFILL the plan-derived affiliate products that are MISSING (kanban t_6d326447).
///
/// **The decision: ONE WRITER.** This route used to be a second, lossy writer of the plan -> product
/// link: its own hand-rolled `INSERT ... VALUES (..., 10.0)` with a LITERAL `default_commission_rate`
/// of 10.0, no `description` and no `category_id`. A plan whose `plans.commission_rate` was 7.5
/// therefore advertised 10.00 through this route and 7.50 through
/// [`crate::handlers::plan_handler::sync_plan_to_affiliate_product`] — the commission a product
/// carried depended on WHICH route created the row. The values are no longer decided here: this
/// route only decides WHEN a product must exist (for a plan that has none) and delegates every
/// column to that helper, which reads name, price, description, category and commission off the
/// `plans` row itself.
///
/// It deliberately does NOT re-rate the products that already exist: rewriting live commission data
/// for every plan in one click is a separate decision (the 6 real plan-derived rows are the evidence
/// that needs it), and a plan edit is what refreshes a product's commercial fields.
pub async fn admin_sync_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let plan_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM plans LIMIT 50")
        .fetch_all(&state.pool)
        .await?;

    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let mut count: i64 = 0;

    for plan_id in plan_ids {
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM affiliate_products WHERE plan_id = $1")
                .bind(plan_id)
                .fetch_one(&state.pool)
                .await?;

        if exists == 0 {
            // ONE WRITER: every column value comes from the plan row. `?` on purpose — a
            // materialisation that failed must not answer 200 with a count that never happened.
            super::plan_handler::sync_plan_to_affiliate_product(
                &state.pool,
                plan_id,
                Some(tenant_id),
            )
            .await?;
            count += 1;
        }
    }

    Ok(Json(
        json!({"synced": count, "message": format!("{} products synced", count)}),
    ))
}

pub async fn admin_update_affiliate_product(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProductRequest>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    update_affiliate_product(auth, State(state), Path(id), Json(req)).await
}

// ── Retired: the cross-app plan -> affiliate-product sync (kanban t_141162e7) ──
//
// `POST /api/v1/internal/sync-affiliate-plan` (`handle_cross_app_plan_sync`) used to live here; its
// ROUTE was deleted from `src/api_router.rs` in the same change. Two recorded decisions, both made
// after this route was designed, leave it with nothing legitimate to do:
//
//   1. ONE WRITER (t_6d326447): the route ended up delegating to
//      `plan_handler::sync_plan_to_affiliate_product`, which reads the plan's own name, price and
//      commission and therefore REQUIRES the plan to exist in THIS service's `plans` table. A
//      sibling app's `plan_id` is a uuid from a different database, so no sibling payload can ever
//      succeed — measured live: a correctly-keyed sibling-shaped post answers
//      `400 plan <uuid> not found` and writes nothing.
//   2. FREE PLANS ONLY (migration 070, David 2026-09-29): `trg_affiliate_products_free_only`
//      RAISES on any affiliate product linked to a paid plan. The catalogue is exactly one
//      `… Free` / `$0` row per `source_app` — seeded by FunnelSwift's own migrations — and the live
//      consumer (`affiliate_tracking_handler::handle_affiliate_upgrade_event`) resolves the product
//      by `source_app`. A sibling has nothing to push: its paid plans may not become affiliate
//      products, and its free row is FunnelSwift-owned.
//
// The catalogue stays FunnelSwift-owned (admin console: POST/PUT/DELETE /api/v1/affiliate-products).
// The three senders (ADASwift, IncentiveSwift, WorkflowSwift) were deleted in the same change.

#[cfg(test)]
mod tests {
    //! Pins the decision `is_category_fk` makes for the TOCTOU backstop (kanban t_45d9684d). The
    //! pre-check inside both writers is what the live probe exercises; the backstop only fires when a
    //! category is deleted BETWEEN the check and the write, which no probe can time, so the mapping
    //! is asserted directly here: the category FK's own SQLSTATE/constraint pair is the field-level
    //! 400, and every neighbouring shape (the table's other FKs, another SQLSTATE, a violation with
    //! no constraint name reported) stays on the generic 500 path.
    use super::is_category_fk;

    const CATEGORY_FK: Option<&str> = Some("affiliate_products_category_id_fkey");

    #[test]
    fn the_category_fk_violation_is_the_apps_own_400() {
        assert!(is_category_fk(Some("23503"), CATEGORY_FK));
    }

    #[test]
    fn every_neighbouring_database_error_keeps_the_generic_500() {
        for (code, constraint) in [
            (Some("23503"), Some("affiliate_products_tenant_id_fkey")),
            (Some("23503"), Some("affiliate_products_plan_id_fkey")),
            (Some("23503"), Some("affiliate_products_system_tag_id_fkey")),
            (Some("23502"), CATEGORY_FK),
            (Some("23505"), CATEGORY_FK),
            (Some("23503"), None),
            (None, CATEGORY_FK),
        ] {
            assert!(
                !is_category_fk(code, constraint),
                "only 23503 on that constraint is a field-level 400: {code:?} {constraint:?}"
            );
        }
    }
}
