//! Commission model endpoints: product groups (one rate for several products), a bulk rate setter,
//! and the resolver that explains which rule produced a rate.
//!
//! Every one of these is reachable from the admin panels — a backend capability with no UI is a
//! defect in this fleet, so each handler here has a matching control (see
//! `www-admin/index.html` and `www-app/index.html`, "Commission groups").

use crate::auth::middleware::AuthUser;
use crate::commission;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

// ── product groups ───────────────────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    /// The one rate that covers every product in the group. NULL is legal and means "this group is
    /// only a label for now" — resolution then falls through to the products' own rates.
    pub commission_rate: Option<f64>,
    pub description: Option<String>,
    /// Products to put in the group at creation time. This is the "selecting which products are the
    /// same" half of the feature: the admin ticks products and gives them one rate.
    pub product_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub commission_rate: Option<f64>,
    pub description: Option<String>,
    /// `true` clears the rate, distinct from omitting the field (`None` = leave alone). Without this,
    /// a rate could be set and never removed — the same one-way trap `clear_system_tag` exists to
    /// avoid on products.
    pub clear_commission_rate: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
pub struct AssignProductsRequest {
    pub product_ids: Vec<Uuid>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkRateRequest {
    pub product_ids: Vec<Uuid>,
    pub commission_rate: f64,
}

#[derive(Debug, serde::Deserialize)]
pub struct ResolveQuery {
    pub product_id: Option<Uuid>,
    pub affiliate_id: Option<String>,
}

pub async fn list_groups(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let rows: Vec<(Uuid, Option<Uuid>, String, Option<f64>, Option<String>, i64)> = sqlx::query_as(
        "SELECT g.id, g.tenant_id, g.name, g.commission_rate::float8, g.description, \
                (SELECT count(*) FROM affiliate_products p WHERE p.group_id = g.id) \
         FROM affiliate_product_groups g ORDER BY g.name ASC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("list groups failed: {e}")))?;

    let out: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.0, "tenant_id": r.1, "name": r.2,
                "commission_rate": r.3, "description": r.4, "product_count": r.5
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

pub async fn create_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateGroupRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if let Some(r) = req.commission_rate {
        if !(0.0..=100.0).contains(&r) {
            return Err(AppError::BadRequest(
                "commission_rate must be between 0 and 100".into(),
            ));
        }
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO affiliate_product_groups (id, name, commission_rate, description) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(req.name.trim())
    .bind(req.commission_rate)
    .bind(&req.description)
    .execute(&state.pool)
    .await?;

    let mut assigned = 0usize;
    if let Some(ids) = &req.product_ids {
        if !ids.is_empty() {
            assigned = sqlx::query("UPDATE affiliate_products SET group_id = $1 WHERE id = ANY($2)")
                .bind(id)
                .bind(ids)
                .execute(&state.pool)
                .await?
                .rows_affected() as usize;
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Group created", "products_assigned": assigned})),
    ))
}

pub async fn update_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateGroupRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    if let Some(r) = req.commission_rate {
        if !(0.0..=100.0).contains(&r) {
            return Err(AppError::BadRequest(
                "commission_rate must be between 0 and 100".into(),
            ));
        }
    }

    // COALESCE-per-field: an omitted field keeps its stored value. `clear_commission_rate` is the
    // explicit way to null the rate, because "send null" and "don't touch it" are the same JSON.
    let res = sqlx::query(
        "UPDATE affiliate_product_groups SET \
            name = COALESCE($2, name), \
            commission_rate = CASE WHEN $5 THEN NULL ELSE COALESCE($3, commission_rate) END, \
            description = COALESCE($4, description), \
            updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(&req.name)
    .bind(req.commission_rate)
    .bind(&req.description)
    .bind(req.clear_commission_rate.unwrap_or(false))
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("group not found".into()));
    }
    Ok(Json(json!({"id": id, "message": "Group updated"})))
}

pub async fn delete_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    // The FK is ON DELETE SET NULL, so member products survive and fall back to their own rates.
    let res = sqlx::query("DELETE FROM affiliate_product_groups WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("group not found".into()));
    }
    Ok(Json(
        json!({"message": "Group deleted; its products keep their own rates"}),
    ))
}

/// Set the membership of a group. **Replaces** the membership, it does not append — so the UI can
/// drive it from a checkbox list where unticking means "remove".
pub async fn assign_products(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignProductsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM affiliate_product_groups WHERE id = $1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(AppError::NotFound("group not found".into()));
    }

    let mut tx = state.pool.begin().await?;
    let removed = sqlx::query("UPDATE affiliate_products SET group_id = NULL WHERE group_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let added = if req.product_ids.is_empty() {
        0
    } else {
        sqlx::query("UPDATE affiliate_products SET group_id = $1 WHERE id = ANY($2)")
            .bind(id)
            .bind(&req.product_ids)
            .execute(&mut *tx)
            .await?
            .rows_affected()
    };
    tx.commit().await?;

    Ok(Json(json!({
        "group_id": id,
        "products_in_group": added,
        "previously_removed": removed,
        "message": "Group membership replaced"
    })))
}

/// Give several products the same rate in one action. This is David's *"do an overall"* without
/// forcing the products into a named group — useful for a one-off correction.
///
/// TWO DEFECTS WERE MEASURED HERE ON 2026-10-02 (kanban t_5c2a9bde) and are now fixed:
///
///   1. **No plan-derived guard.** `UPDATE ... WHERE id = ANY($1)` could re-rate a plan-derived
///      product (`plan_id IS NOT NULL`), whose rate IS its plan's rate (t_92bd5eb6, migration 076). The
///      resolver (`src/commission.rs`) ranks the product's own column ABOVE the plan's and the
///      conversion receiver calls it, so one click here changed what a sale PAID — and the next plan
///      save silently changed it back, leaving no record of which value was in force. The route now
///      REFUSES the whole request with a 409 naming the plan to edit.
///   2. **No tenant filter.** The predicate was `id = ANY($1)` alone, so an admin could re-rate any
///      workspace's product by id. The scope is now the admin product list's own contract — the
///      caller's products OR the fleet's SYSTEM tenant's (where the plan-derived rows and the sibling
///      apps' free products live) — and the ids it could not reach are reported back as
///      `products_skipped` instead of being silently dropped.
pub async fn bulk_set_product_rate(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BulkRateRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    if !(0.0..=100.0).contains(&req.commission_rate) {
        return Err(AppError::BadRequest(
            "commission_rate must be between 0 and 100".into(),
        ));
    }
    if req.product_ids.is_empty() {
        return Err(AppError::BadRequest("product_ids must not be empty".into()));
    }
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // One row per id, so `products_skipped` counts rows rather than repeat ticks.
    let mut wanted: Vec<Uuid> = Vec::with_capacity(req.product_ids.len());
    for id in &req.product_ids {
        if !wanted.contains(id) {
            wanted.push(*id);
        }
    }
    // The rows this call may reach. No tenant predicate at all until now; this is the list's own scope.
    let rows: Vec<(
        Uuid,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        Option<f64>,
    )> = sqlx::query_as(
        "SELECT ap.id, ap.plan_id, ap.name, p.name, p.commission_rate::float8 \
               FROM affiliate_products ap LEFT JOIN plans p ON p.id = ap.plan_id \
              WHERE ap.id = ANY($1) AND (ap.tenant_id = $2 OR ap.tenant_id = $3)",
    )
    .bind(&wanted)
    .bind(tenant_id)
    .bind(crate::system_tenant::system_tenant_id())
    .fetch_all(&state.pool)
    .await?;

    // A plan-derived product's rate IS its plan's rate: refuse the WHOLE request rather than skipping
    // the plan-derived ids. A partial write is exactly the invisible drift this card is about — the
    // operator would read "Rate set on 3 products" while the money path paid a value nobody chose.
    // The wording matches `affiliate_product_handler::plan_derived_rate_conflict`, which answers the
    // single-row 409 the console's own form gets.
    let derived: Vec<&(
        Uuid,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        Option<f64>,
    )> = rows.iter().filter(|r| r.1.is_some()).collect();
    if !derived.is_empty() {
        let named: Vec<String> = derived
            .iter()
            .take(3)
            .map(|r| {
                let name = r.2.as_deref().unwrap_or("unnamed");
                match (r.3.as_deref(), r.4) {
                    (Some(plan), Some(plan_rate)) => {
                        format!("\"{name}\" (from its plan \"{plan}\" at {plan_rate}%)")
                    }
                    (Some(plan), None) => format!("\"{name}\" (from its plan \"{plan}\")"),
                    _ => format!("\"{name}\" (from its plan)"),
                }
            })
            .collect();
        let more = if derived.len() > 3 {
            format!(" and {} more", derived.len() - 3)
        } else {
            String::new()
        };
        return Err(AppError::Conflict(format!(
            "The commission rate for {}{more} comes from the plan it belongs to — a plan-derived \
             product is a mirror of its plan and has no rate of its own, so edit the plan to change \
             the rate (or untick the product).",
            named.join(", ")
        )));
    }

    // `plan_id IS NULL` is repeated in the UPDATE so the statement itself can never drift a
    // plan-derived row, even if one became plan-derived between the read above and this write (that
    // row then counts as skipped below).
    let res = sqlx::query(
        "UPDATE affiliate_products SET default_commission_rate = $2, updated_at = now() \
          WHERE id = ANY($1) AND (tenant_id = $3 OR tenant_id = $4) AND plan_id IS NULL",
    )
    .bind(&wanted)
    .bind(req.commission_rate)
    .bind(tenant_id)
    .bind(crate::system_tenant::system_tenant_id())
    .execute(&state.pool)
    .await?;

    let updated = res.rows_affected();
    Ok(Json(json!({
        "products_updated": updated,
        // Ticked ids this call could not reach: another workspace's product, an unknown id, or (in a
        // race) a row that became plan-derived between the read and this UPDATE. Reported rather than
        // silently dropped, because "Rate set on N products" has to BE N.
        "products_skipped": (wanted.len() as u64).saturating_sub(updated),
        "commission_rate": req.commission_rate,
        "message": "Rate applied to the selected products"
    })))
}

/// Which rule wins, and what it beat. Powers the admin panel's "effective rate" display so a number
/// on screen can always be explained.
pub async fn resolve_rate(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ResolveQuery>,
) -> AppResult<Json<serde_json::Value>> {
    if q.product_id.is_none() && q.affiliate_id.is_none() {
        return Err(AppError::BadRequest(
            "supply product_id and/or affiliate_id".into(),
        ));
    }
    let resolved = commission::resolve(&state.pool, q.product_id, q.affiliate_id.as_deref()).await;
    Ok(Json(json!(resolved)))
}

/// Preview the money for an amount at the resolved rate.
pub async fn preview_commission(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<PreviewQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let resolved = commission::resolve(&state.pool, q.product_id, q.affiliate_id.as_deref()).await;
    let amount = commission::commission_for(q.amount, resolved.rate);
    Ok(Json(json!({
        "amount": q.amount,
        "rate": resolved.rate,
        "source": resolved.source,
        "explanation": resolved.explanation,
        "commission": amount,
        "considered": resolved.considered,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct PreviewQuery {
    pub amount: f64,
    pub product_id: Option<Uuid>,
    pub affiliate_id: Option<String>,
}
