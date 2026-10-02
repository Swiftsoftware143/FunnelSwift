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

// ── THE 075 TAG RULE, made explainable (kanban t_c149b025) ────────────────────────────────────────
//
// `trg_one_active_product_per_tag` (migration 075, t_68e92c8e) already REFUSES a second ACTIVE
// product on an already-routed system tag and NAMES the conflict in its message — but that message
// never reached the operator. The trigger is a plpgsql `RAISE EXCEPTION`, so it arrives as
// `sqlx::Error::Database`, and `AppError::Database` maps that (src/error.rs) to
// `500 {"error":"Database error"}` while the real text only went to `tracing::error!`. Both writers
// below bound the form's `system_tag_id` straight into the INSERT/UPDATE, so an admin who picked a
// tag another product already owned got an unexplained 500 — the fleet's "a control the panel
// cannot explain" class.
//
// ARM (b) — the trigger's OWN predicate as a READ, before the write: `ensure_tag_route_free` mirrors
// the trigger's fired-condition and its row selection, so the panel gets a `409` naming the holder
// (the admin console's `api()` throws `e.error` and `openAffiliateProductModal`'s catch renders it
// as a toast). ARM (a) is kept as the TOCTOU BACKSTOP, exactly as this file already does for
// `category_id` (`category_fk_error`): the trigger's own raise is recognised and answers the SAME
// 409 instead of the generic 500, so a tag taken between the read and the write is still explained.

/// The name of the ACTIVE product that already routes `tag_id`, if any — literally the trigger's own
/// row selection (`WHERE p.system_tag_id = NEW.system_tag_id AND p.is_active AND p.id <> NEW.id`),
/// with a deterministic `ORDER BY` added because the trigger's own `LIMIT 1` has none.
///
/// DELIBERATELY NOT tenant-scoped: system tags are fleet-wide rows (`tags.is_system`, owned by the
/// system tenant) and the trigger is global, so a tenant-scoped read would answer "free" for a tag
/// another workspace already routes and hand the admin the same unexplained 500 from the write.
async fn route_holder_name(
    state: &AppState,
    tag_id: Uuid,
    row_id: Uuid,
) -> AppResult<Option<String>> {
    let holder: Option<String> = sqlx::query_scalar(
        "SELECT name FROM affiliate_products
          WHERE system_tag_id = $1 AND is_active AND id <> $2
          ORDER BY created_at, id
          LIMIT 1",
    )
    .bind(tag_id)
    .bind(row_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(holder)
}

/// The 409 body for the tag rule. ONE message for both arms, so the panel cannot tell which one
/// refused it — and the named holder is always a row that really holds the tag (the backstop
/// re-resolves it with the same read rather than parsing the trigger's text).
fn tag_already_routed(holder: Option<&str>) -> AppError {
    match holder {
        Some(holder) => AppError::Conflict(format!(
            "That system tag is already routing \"{holder}\" — one tag routes one product. \
             Retire that product first, or pick another tag."
        )),
        None => AppError::Conflict(
            "That system tag already routes another active product — one tag routes one product. \
             Retire that product first, or pick another tag."
                .into(),
        ),
    }
}

/// ARM (b): refuse BEFORE the write, naming the conflict. Mirrors the trigger's fired-condition
/// exactly — a retired row (`is_active` false), no tag, or an UPDATE that was ALREADY active on the
/// same stored tag (the trigger's `OLD.is_active IS TRUE AND tag unchanged` early return) is NOT a
/// conflict — so this read can never refuse something the database would have accepted.
///
/// The `stored_active` argument is what makes this mirror WIDENED migration 087 rather than 075: an
/// UPDATE that does not change the tag but DOES flip `is_active` false -> true is an ACTIVATION, not
/// a no-op, so it is checked. `stored_active` is `false` on the create path (there is no stored row).
async fn ensure_tag_route_free(
    state: &AppState,
    tag_id: Option<Uuid>,
    row_id: Uuid,
    stored_tag_id: Option<Uuid>,
    stored_active: bool,
    is_active: bool,
) -> AppResult<()> {
    if !is_active {
        return Ok(());
    }
    let Some(tag_id) = tag_id else { return Ok(()) };
    if stored_active && stored_tag_id == Some(tag_id) {
        return Ok(());
    }
    if let Some(holder) = route_holder_name(state, tag_id, row_id).await? {
        return Err(tag_already_routed(Some(&holder)));
    }
    Ok(())
}

/// SQLSTATE of every plpgsql `RAISE EXCEPTION` that does not set an explicit `ERRCODE`.
const RAISE_EXCEPTION_SQLSTATE: &str = "P0001";

/// A stable fragment of the 075 trigger's message. Matching on text is safe HERE because migration
/// 075 is APPLIED: sqlx records a checksum and refuses a changed migration file at boot (fleet doc
/// §8.3), so these bytes cannot move without that refusal being visible. Every OTHER raise in this
/// database — including migration 070's paid-plan refusal
/// (`trg_affiliate_products_free_only`) — keeps the generic 500 path.
const TAG_RULE_MESSAGE: &str = "cannot be routed by that system tag";

/// The decision the backstop makes, split out so it can be asserted without faking a driver error:
/// ONLY SQLSTATE P0001 (raise_exception) AND ONLY the 075 rule's own message.
fn is_tag_rule_raise(code: Option<&str>, message: &str) -> bool {
    code == Some(RAISE_EXCEPTION_SQLSTATE) && message.contains(TAG_RULE_MESSAGE)
}

/// SQLSTATE of a unique-index violation.
const UNIQUE_VIOLATION_SQLSTATE: &str = "23505";

/// Migration 087's partial unique index — the arm with the database-level guarantee. It sits BEHIND
/// the trigger (a BEFORE ROW trigger runs before the heap write and its index checks), so it is only
/// reached when the trigger's own read saw no holder: two writers taking the same tag at the same
/// instant, where the losing statement gets a `23505` here instead of the trigger's `P0001`.
const TAG_ROUTE_UNIQUE_INDEX: &str = "uq_affiliate_products_active_tag";

/// Recognised by SQLSTATE AND CONSTRAINT NAME, not by text: no other unique index in this database
/// can be mistaken for 087's, and 087's index cannot be mistaken for anything else.
fn is_tag_route_index_violation(code: Option<&str>, constraint: Option<&str>) -> bool {
    code == Some(UNIQUE_VIOLATION_SQLSTATE) && constraint == Some(TAG_ROUTE_UNIQUE_INDEX)
}

// ── THE t_92bd5eb6 RULE AT THE WRITE PATH (kanban t_5c2a9bde) ────────────────────────────────────
//
// t_92bd5eb6 decided, and migration 076 normalised, that a plan-derived affiliate product
// (`plan_id IS NOT NULL`) carries no rate of its own: `plans.commission_rate` IS its
// `default_commission_rate`. The decision was recorded but NOT enforced, and the value it protects is
// money: `src/commission.rs` ranks the product's own column ABOVE the plan's and the authoritative
// conversion receiver calls it, so whatever sits in `default_commission_rate` is what a sale PAYS.
// Two writers could still move it (the bulk rate route, with no tenant filter and no plan-derived
// guard, and `PUT /api/v1/affiliate-products/:id`), and the drift was invisible AND self-erasing — the
// next plan save rewrote the value, so the paid rate changed and then silently changed back.
//
// ARM (b) — the rule as a READ, before the write: `resolve_plan_derived_rate` refuses a rate change on
// a plan-derived row with a 409 that names the plan to edit, and for a caller that asks for nothing (a
// rename, a tag change) it binds the plan's own rate, so a save can never leave the mirror out of step.
//
// ARM (a) — the TOCTOU backstop: migration 083's trigger refuses the same write in the DATABASE
// (SQLSTATE `SW001`, message naming the product and the plan) and `map_product_write_error` recognises
// that raise and answers the SAME 409 instead of `error.rs`'s anonymous `500 "Database error"` — the
// class t_c149b025 fixed for the 075 tag rule. The backstop is reachable: a plan edited between the
// read above and the write leaves the bound value stale.

/// SQLSTATE migration 083's trigger raises with. Class `SW` is user-defined, so it cannot collide with
/// a PostgreSQL or SQL-standard code — `P0001` (the unnamed plpgsql raise) is NOT usable here, because
/// 070's paid-plan refusal and 075's tag rule both raise from it.
const PLAN_RATE_SQLSTATE: &str = "SW001";

/// A stable fragment of migration 083's raise. Matching on text is safe HERE for the same reason 075's
/// is: sqlx records a checksum and refuses a changed migration file at boot, so these bytes cannot
/// move without that refusal being visible.
const PLAN_RATE_MESSAGE: &str = "commission rate on affiliate product";

/// The plan a product derives its rate from. `None` from [`plan_rate_source`] means the product is not
/// plan-derived (its `plan_id` is NULL), which is the only fact this rule needs.
pub(crate) struct PlanRateSource {
    pub product_name: Option<String>,
    pub plan_name: Option<String>,
    pub plan_rate: Option<f64>,
}

/// The plan behind `product_id`, or `None` when the product has no plan. A missing product also
/// answers `None`: every caller has already resolved the row it is writing.
pub(crate) async fn plan_rate_source(
    state: &AppState,
    product_id: Uuid,
) -> AppResult<Option<PlanRateSource>> {
    let row: Option<(Option<String>, Option<Uuid>, Option<String>, Option<f64>)> = sqlx::query_as(
        "SELECT ap.name, ap.plan_id, p.name, p.commission_rate::float8 \
           FROM affiliate_products ap LEFT JOIN plans p ON p.id = ap.plan_id \
          WHERE ap.id = $1",
    )
    .bind(product_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(
        row.and_then(|(product_name, plan_id, plan_name, plan_rate)| {
            plan_id.map(|_| PlanRateSource {
                product_name,
                plan_name,
                plan_rate,
            })
        }),
    )
}

/// The 409 an operator sees when something tries to write a plan-derived product's rate. ONE message
/// for arm (b) and arm (a), so the caller cannot tell which one refused it, and it names the plan to
/// edit rather than the column that disagreed.
pub(crate) fn plan_derived_rate_conflict(
    product_name: &str,
    plan_name: Option<&str>,
    plan_rate: Option<f64>,
) -> AppError {
    let from = match (plan_name, plan_rate) {
        (Some(plan), Some(rate)) => format!("its plan \"{plan}\" ({rate}%)"),
        (Some(plan), None) => format!("its plan \"{plan}\""),
        _ => "the plan it belongs to".to_string(),
    };
    AppError::Conflict(format!(
        "The commission rate for \"{product_name}\" comes from {from} — this product is a mirror of \
         its plan and has no rate of its own, so edit the plan to change the rate."
    ))
}

/// ARM (b). `requested` is what the caller asked for, if anything: asking for the plan's own rate (or
/// asking for nothing) writes the plan's rate back, while asking for any OTHER number is refused. The
/// "asked for nothing" arm is what heals a legacy drifted row on an unrelated save instead of letting
/// the trigger refuse a rename.
fn resolve_plan_derived_rate(src: &PlanRateSource, requested: Option<f64>) -> AppResult<f64> {
    let name = src.product_name.as_deref().unwrap_or("this product");
    let Some(plan_rate) = src.plan_rate else {
        return Err(plan_derived_rate_conflict(
            name,
            src.plan_name.as_deref(),
            None,
        ));
    };
    match requested {
        Some(want) if want != plan_rate => Err(plan_derived_rate_conflict(
            name,
            src.plan_name.as_deref(),
            src.plan_rate,
        )),
        _ => Ok(plan_rate),
    }
}

/// The decision the backstop makes, split out so it can be asserted without faking a driver error:
/// ONLY SQLSTATE `SW001` AND ONLY the 083 rule's own message.
fn is_plan_rate_raise(code: Option<&str>, message: &str) -> bool {
    code == Some(PLAN_RATE_SQLSTATE) && message.contains(PLAN_RATE_MESSAGE)
}

/// ARM (a), the backstop `ensure_tag_route_free` cannot cover itself: another writer takes the tag
/// between that read and this write, so the trigger fires. The admin must still get the SAME 409.
/// Migration 087 adds a SECOND refusal behind the trigger — its partial unique index — so a race that
/// slips past the trigger's own read is answered with the same 409 too.
async fn map_product_write_error(
    state: &AppState,
    e: sqlx::Error,
    tag_id: Option<Uuid>,
    row_id: Uuid,
) -> AppError {
    if let sqlx::Error::Database(db) = &e {
        let code = db.code();
        let code = code.as_deref();
        if is_tag_rule_raise(code, db.message())
            || is_tag_route_index_violation(code, db.constraint())
        {
            let holder = match tag_id {
                Some(tag_id) => route_holder_name(state, tag_id, row_id)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            return tag_already_routed(holder.as_deref());
        }
        // ARM (a) of the t_92bd5eb6 rule (kanban t_5c2a9bde): migration 083's trigger refused a rate
        // that would leave this row out of step with its plan. The plan is re-resolved so the 409 names
        // it (the trigger's own text stays in the server log, exactly like 075's), and the answer is the
        // SAME 409 the pre-write read answers.
        if is_plan_rate_raise(code, db.message()) {
            let src = plan_rate_source(state, row_id).await.ok().flatten();
            return match src {
                Some(src) => plan_derived_rate_conflict(
                    src.product_name.as_deref().unwrap_or("this product"),
                    src.plan_name.as_deref(),
                    src.plan_rate,
                ),
                None => AppError::Conflict(
                    "That product's commission rate comes from the plan it belongs to — this product \
                     is a mirror of its plan and has no rate of its own, so edit the plan to change \
                     the rate."
                        .into(),
                ),
            };
        }
    }
    category_fk_error(e)
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
    // ARM (b) of the 075/087 tag rule (kanban t_c149b025, t_5196edda): the form's tag choice is
    // checked against the trigger's own predicate BEFORE the write, so an admin who picked a tag
    // another active product already routes gets a 409 naming the holder instead of the trigger's
    // unexplained 500. Create path: there is no stored row, so `stored_active` is false.
    ensure_tag_route_free(&state, req.system_tag_id, id, None, false, is_active).await?;

    let written = sqlx::query(
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
    .await;
    // ARM (a): the trigger is the backstop for a tag taken between the read above and this write.
    // It answers the SAME 409 (the category FK's own mapping is kept inside the mapper).
    if let Err(e) = written {
        return Err(map_product_write_error(&state, e, req.system_tag_id, id).await);
    }

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
    // The t_92bd5eb6 rule (kanban t_5c2a9bde). `plan_rate_source` answers `Some` only for a
    // plan-derived row (`plan_id IS NOT NULL`), and for one the PLAN's rate is what gets written
    // whatever the caller asked: a different number is refused with a 409 naming the plan, and no
    // number at all is healed to it. A row with no plan keeps the caller's value, as before.
    let plan_source = plan_rate_source(&state, id).await?;
    let commission = match &plan_source {
        Some(src) => resolve_plan_derived_rate(src, req.default_commission_rate)?,
        None => req.default_commission_rate.unwrap_or(existing.3),
    };
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
    // ARM (b) of the 075/087 tag rule (kanban t_c149b025, t_5196edda), before the write and against
    // the EFFECTIVE values: the check is skipped when the row is being retired, when no tag is set,
    // and — exactly like the trigger's own WIDENED early return — only when the row was ALREADY
    // active on the same stored tag. A re-activation (`is_active` false -> true with the tag kept) is
    // therefore checked, which is the hole this card measured: it used to short-circuit here and in
    // the trigger and leave the tag routing two active products.
    ensure_tag_route_free(
        &state,
        system_tag_id,
        id,
        existing.9,
        existing.10,
        is_active,
    )
    .await?;

    let written = sqlx::query(
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
    .await;
    // ARM (a): the trigger's own raise answers the SAME 409 (the category FK's mapping is kept).
    if let Err(e) = written {
        return Err(map_product_write_error(&state, e, system_tag_id, id).await);
    }

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

/// A stable fragment of migration 070's paid-plan refusal (`trg_affiliate_products_free_only`).
/// Matching on text is safe for the same reason 075's and 083's markers are: sqlx records a checksum
/// and refuses a changed migration file at boot, so these bytes cannot move without that refusal
/// being visible.
const FREE_ONLY_RAISE_MESSAGE: &str = "may not be linked to a PAID plan";

/// The decision the backfill's TOCTOU backstop makes, split out so it can be asserted without faking
/// a driver error: ONLY SQLSTATE P0001 (an unnamed plpgsql `RAISE EXCEPTION`) AND ONLY migration
/// 070's own message. 075's tag rule raises from the same class and is deliberately left alone.
fn is_free_only_raise_parts(code: Option<&str>, message: &str) -> bool {
    code == Some(RAISE_EXCEPTION_SQLSTATE) && message.contains(FREE_ONLY_RAISE_MESSAGE)
}

/// The same decision applied to the error a materialisation actually returned — the helper answers
/// `AppError`, so the driver error is unwrapped through the arm `error.rs` would have mapped to its
/// anonymous 500.
fn is_free_only_raise(e: &AppError) -> bool {
    match e {
        AppError::Database(sqlx::Error::Database(db)) => {
            is_free_only_raise_parts(db.code().as_deref(), db.message())
        }
        _ => false,
    }
}

/// ONE shape for a plan the backfill left alone, so both arms report it identically. `price` is
/// `None` on the TOCTOU arm, where the plan was priced up between the read and the INSERT and
/// re-reading it would only race again.
fn skipped_plan(id: Uuid, name: &str, price: Option<f64>) -> Value {
    json!({"id": id, "name": name, "price": price})
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
///
/// **AFFILIATES HAND OUT FREE PLANS ONLY (migration 070, David 2026-09-29) — kanban t_a88f6d66.**
/// `trg_affiliate_products_free_only` REFUSES an affiliate product linked to a plan whose
/// `price <> 0`, and migration 074 removed the paid-plan catalogue rows, so a walk that materialises
/// a product for EVERY productless plan raised on the first paid one. That raise is
/// `sqlx::Error::Database` and `error.rs` maps it to `500 {"error":"Database error"}`, so on a
/// catalogue with 4 paid plans and 2 free ones this route answered a bare 500 and could never
/// backfill anything — while the trigger's own text stayed in the log.
///
/// **ARM (a) — skip and REPORT, decided over arm (b) (refuse the request).** The trigger's own
/// fired-condition (`price IS NOT NULL AND price <> 0`) is mirrored here as a READ: a plan the
/// database may not link to a product is skipped, and the answer names every one of them
/// (`skipped` + `skipped_plans` with id, name and price). The route's own contract — "materialise
/// the MISSING ones" — stays true for the only plans an affiliate may hand out, and the panel can
/// explain what it left alone. Arm (b) is not available here: unlike the bulk rate route
/// (t_5c2a9bde), this route names NO plan, so a 4xx would make the backfill permanently unusable for
/// as long as any paid plan exists — it would refuse the FREE plan it can still materialise. A
/// fleet-wide backfill reports what it left alone; it does not fail.
///
/// The race between that read and the INSERT is the TOCTOU backstop: the trigger's raise is
/// recognised by [`is_free_only_raise`] and counted as skipped too, so the trigger's text never
/// reaches the caller as "Database error".
///
/// **The product's OWNER is not this route's decision (kanban t_3152f9ba).** This route used to pass
/// `Some(auth.tenant_id)` — the *signed-in admin's* workspace — so the owner of a platform-level
/// plan's product depended on which admin happened to press Backfill. Measured live: the catalogue
/// is system-tenant while the admin sits in their own workspace, so a customer-workspace admin would
/// have stamped the fleet's shared rows with THEIR tenant. That argument is gone; the helper owns
/// the owner, and there is exactly one value it can write.
pub async fn admin_sync_affiliate_products(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    // `name` and `price` ride along so the answer can name what it left alone. `plans.price` is
    // `double precision`, cast for symmetry with the other plan reads in this service.
    let plans: Vec<(Uuid, String, f64)> = sqlx::query_as(
        "SELECT id, name, price::float8 FROM plans ORDER BY created_at, id LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut synced: i64 = 0;
    let mut skipped: Vec<Value> = Vec::new();

    for (plan_id, plan_name, price) in plans {
        // The free-plan rule (migration 070) as a READ. A paid plan can never own an affiliate
        // product, so it is REPORTED rather than left to raise: the trigger is the authority, and
        // this mirrors its fired-condition exactly, so this can never skip something the database
        // would have accepted. `=` on the exact literal the trigger compares.
        if price != 0.0 {
            skipped.push(skipped_plan(plan_id, &plan_name, Some(price)));
            continue;
        }

        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM affiliate_products WHERE plan_id = $1")
                .bind(plan_id)
                .fetch_one(&state.pool)
                .await?;

        if exists == 0 {
            // ONE WRITER: every column value comes from the plan row — including the product's
            // OWNER, which the helper decides (t_3152f9ba) and this route does not pass at all.
            // Not `?` — a TOCTOU refusal must not answer 500, and a real failure must not answer 200
            // with a count that never happened.
            match super::plan_handler::sync_plan_to_affiliate_product(&state.pool, plan_id).await {
                Ok(_) => synced += 1,
                // The plan was priced up between the read above and the INSERT. The trigger is the
                // authority; report it the same way as the pre-filter instead of raising.
                Err(e) if is_free_only_raise(&e) => {
                    skipped.push(skipped_plan(plan_id, &plan_name, None));
                }
                Err(e) => return Err(e),
            }
        }
    }

    let message = if skipped.is_empty() {
        format!("{} products synced", synced)
    } else {
        format!(
            "{} products synced; {} paid plan(s) skipped (affiliates only hand out free plans)",
            synced,
            skipped.len()
        )
    };
    Ok(Json(json!({
        "synced": synced,
        "skipped": skipped.len(),
        "skipped_plans": skipped,
        "message": message,
    })))
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

    // ── the 075 tag rule's backstop (kanban t_c149b025) ──────────────────────────────────────────
    //
    // `ensure_tag_route_free` (arm b) is what the live probe exercises; the mapping is asserted here
    // for the TOCTOU case no probe can time — another writer takes the tag between the read and the
    // write, so the trigger fires and the admin must still get the SAME 409.

    use super::{is_tag_rule_raise, tag_already_routed, TAG_RULE_MESSAGE};

    /// The real message migration 075's trigger raises (`%` filled in), byte for byte minus the
    /// interpolated names.
    const TRIGGER_MESSAGE: &str =
        "affiliate product \"Probe B\" cannot be routed by that system tag: \
                                   product \"Probe A\" already is. One tag routes one product — \
                                   retire that product first, or use another tag.";

    /// Migration 070's own refusal (`trg_affiliate_products_free_only`) — the neighbouring raise
    /// this mapping must NOT swallow, measured from that migration file.
    const FREE_ONLY_MESSAGE: &str = "affiliate product \"Probe B\" may not be linked to a PAID plan \
                                     (price 49). Affiliates may only hand out free plans; the customer \
                                     upgrades in app and pays for it themselves.";

    #[test]
    fn the_tag_rule_raise_is_the_apps_own_409() {
        assert!(is_tag_rule_raise(Some("P0001"), TRIGGER_MESSAGE));
        // The trigger's text is the discriminator, not the class: 070 raises from the same SQLSTATE.
        assert!(!is_tag_rule_raise(Some("P0001"), FREE_ONLY_MESSAGE));
        assert!(!is_tag_rule_raise(None, TRIGGER_MESSAGE));
        assert!(!is_tag_rule_raise(Some("23505"), TRIGGER_MESSAGE));
        assert!(!is_tag_rule_raise(Some("P0001"), "some other failure"));
        // The marker is the whole contract, so assert it against the REAL trigger text rather than
        // against itself: if a migration reworded the raise, this is the assertion that fails.
        assert!(is_tag_rule_raise(Some("P0001"), TRIGGER_MESSAGE));
    }

    #[test]
    fn the_409_names_the_holder_and_never_implies_a_generic_failure() {
        let named = tag_already_routed(Some("Probe A"));
        let text = named.to_string();
        assert!(text.contains("Probe A"), "the holder must be named: {text}");
        assert!(text.contains("one tag routes one product"));
        assert!(matches!(named, crate::error::AppError::Conflict(_)));

        // The fallback (the holder vanished between the raise and the re-read) keeps the ADVICE and
        // never leaks the database's own text into the panel.
        let unnamed = tag_already_routed(None).to_string();
        assert!(unnamed.contains("Retire that product first, or pick another tag"));
        assert!(!unnamed.contains(TAG_RULE_MESSAGE));
        assert!(matches!(
            tag_already_routed(None),
            crate::error::AppError::Conflict(_)
        ));
    }

    // ── migration 087's index, the SECOND refusal behind the trigger (kanban t_5196edda) ─────────

    use super::{is_tag_route_index_violation, TAG_ROUTE_UNIQUE_INDEX};

    #[test]
    fn the_087_index_violation_is_the_apps_own_409() {
        // Exactly 23505 on exactly 087's index.
        assert!(is_tag_route_index_violation(
            Some("23505"),
            Some(TAG_ROUTE_UNIQUE_INDEX)
        ));
        // The 075 index (source_app keys) is a DIFFERENT rule and must keep its own path.
        assert!(!is_tag_route_index_violation(
            Some("23505"),
            Some("uq_affiliate_products_active_key")
        ));
        // Not a unique violation at all, or the constraint name is missing.
        assert!(!is_tag_route_index_violation(
            Some("P0001"),
            Some(TAG_ROUTE_UNIQUE_INDEX)
        ));
        assert!(!is_tag_route_index_violation(Some("23505"), None));
        assert!(!is_tag_route_index_violation(
            None,
            Some(TAG_ROUTE_UNIQUE_INDEX)
        ));
        // The name is the whole contract, so assert it against the REAL definition, not itself: if a
        // migration renamed the index without moving this constant, this is the assertion that fails.
        let migration = include_str!(
            "../../migrations/087_one_active_product_per_system_tag_covers_reactivation.sql"
        );
        assert!(
            migration.contains(TAG_ROUTE_UNIQUE_INDEX),
            "migration 087 must define the index this mapping recognises"
        );
    }

    // ── the t_92bd5eb6 rule's backstop (kanban t_5c2a9bde) ──────────────────────────────────────
    //
    // The live probe exercises arm (b), the read before the write. This mapping is asserted here for
    // the TOCTOU case no probe can time: the plan's rate moves between that read and the write, so
    // migration 083's trigger fires and the admin must still get the SAME 409 instead of a bare 500.

    use super::{
        is_plan_rate_raise, plan_derived_rate_conflict, resolve_plan_derived_rate, PlanRateSource,
        PLAN_RATE_MESSAGE, PLAN_RATE_SQLSTATE,
    };

    /// The raise migration 083's trigger produces for one real live row (`%` filled in).
    const TRIGGER_RATE_MESSAGE: &str =
        "the commission rate on affiliate product \"FunnelSwift Capture Free\" comes from its plan \
         \"Capture Free\" (20.00%) - this product mirrors its plan and has no rate of its own, so \
         edit the plan to change the rate";

    fn plan_source(plan: Option<&str>, rate: Option<f64>) -> PlanRateSource {
        PlanRateSource {
            product_name: Some("Probe Product".to_string()),
            plan_name: plan.map(str::to_string),
            plan_rate: rate,
        }
    }

    #[test]
    fn the_plan_rate_raise_is_the_apps_own_409() {
        // The trigger's SQLSTATE plus its own message is the rule...
        assert!(is_plan_rate_raise(
            Some(PLAN_RATE_SQLSTATE),
            TRIGGER_RATE_MESSAGE
        ));
        // ...and the marker is asserted against the REAL trigger text rather than against itself: if a
        // migration reworded the raise, this is the assertion that fails.
        assert!(TRIGGER_RATE_MESSAGE.contains(PLAN_RATE_MESSAGE));
        // 070 and 075 both raise from P0001, so the SQLSTATE alone is never enough, and this rule never
        // swallows a neighbouring raise.
        assert!(!is_plan_rate_raise(Some("P0001"), TRIGGER_RATE_MESSAGE));
        assert!(!is_plan_rate_raise(
            Some(PLAN_RATE_SQLSTATE),
            FREE_ONLY_MESSAGE
        ));
        assert!(!is_plan_rate_raise(None, TRIGGER_RATE_MESSAGE));
        assert!(!is_plan_rate_raise(
            Some(PLAN_RATE_SQLSTATE),
            "some other failure"
        ));
    }

    #[test]
    fn a_plan_derived_rate_is_only_ever_the_plans_own() {
        let src = plan_source(Some("Capture Free"), Some(20.0));
        // Asking for exactly the plan's rate, or not asking for one at all, writes the plan's rate.
        assert_eq!(resolve_plan_derived_rate(&src, Some(20.0)).unwrap(), 20.0);
        assert_eq!(resolve_plan_derived_rate(&src, None).unwrap(), 20.0);
        // Any other number is precisely the drift this card is about: refused, naming the plan.
        let refused = resolve_plan_derived_rate(&src, Some(10.0)).unwrap_err();
        let text = refused.to_string();
        assert!(
            text.contains("Capture Free"),
            "the plan must be named: {text}"
        );
        assert!(
            text.contains("20"),
            "the rate to keep must be named: {text}"
        );
        assert!(text.contains("edit the plan"));
        assert!(matches!(refused, crate::error::AppError::Conflict(_)));
        // A plan-derived row whose plan row is gone (unreachable behind the plan_id FK) is refused
        // rather than inheriting whatever the column happens to hold.
        assert!(matches!(
            resolve_plan_derived_rate(&plan_source(None, None), None),
            Err(crate::error::AppError::Conflict(_))
        ));
    }

    // ── the 070 free-plan rule's backstop (kanban t_a88f6d66) ───────────────────────────────────
    //
    // The live probe exercises arm (a), the read before the write. This mapping is asserted here for
    // the TOCTOU case no probe can time: the plan's price moves between that read and the INSERT, so
    // migration 070's trigger fires and the route must still REPORT the plan instead of answering
    // `500 "Database error"`.

    use super::{is_free_only_raise_parts, FREE_ONLY_RAISE_MESSAGE};

    #[test]
    fn the_paid_plan_raise_is_recognised_and_never_becomes_a_500() {
        assert!(is_free_only_raise_parts(Some("P0001"), FREE_ONLY_MESSAGE));
        // The marker is asserted against the REAL trigger text rather than against itself: if a
        // migration reworded the raise, this is the assertion that fails.
        assert!(FREE_ONLY_MESSAGE.contains(FREE_ONLY_RAISE_MESSAGE));
        // 075's tag rule raises from the same class, so the marker is what separates them, and this
        // mapping never swallows a neighbouring raise.
        assert!(!is_free_only_raise_parts(Some("P0001"), TRIGGER_MESSAGE));
        assert!(!is_free_only_raise_parts(
            Some("P0001"),
            TRIGGER_RATE_MESSAGE
        ));
        assert!(!is_free_only_raise_parts(Some("SW001"), FREE_ONLY_MESSAGE));
        assert!(!is_free_only_raise_parts(Some("23505"), FREE_ONLY_MESSAGE));
        assert!(!is_free_only_raise_parts(None, FREE_ONLY_MESSAGE));
        assert!(!is_free_only_raise_parts(
            Some("P0001"),
            "some other failure"
        ));
    }

    #[test]
    fn the_rate_409_never_leaks_the_databases_own_text() {
        let named = plan_derived_rate_conflict("Probe Product", Some("Capture Free"), Some(20.0));
        assert!(matches!(named, crate::error::AppError::Conflict(_)));
        let named_text = named.to_string();
        assert!(named_text.contains("Probe Product") && named_text.contains("Capture Free"));
        // The fallback (the plan vanished between the raise and the re-read) keeps the ADVICE and never
        // leaks the database's own text into the panel.
        let unnamed = plan_derived_rate_conflict("Probe Product", None, None).to_string();
        assert!(unnamed.contains("edit the plan to change the rate"));
        assert!(!unnamed.contains(TRIGGER_RATE_MESSAGE));
    }
}
