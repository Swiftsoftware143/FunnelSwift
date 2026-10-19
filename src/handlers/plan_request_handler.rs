//! Plan requests — a workspace ASKS to be moved onto a paid plan.
//!
//! WHY THIS MODULE EXISTS (measured live 2026-10-19): this install has no payment provider
//! configured, so `POST /api/v1/checkout/create` refuses every purchase (503
//! `payment_provider_not_configured`). The Upgrade screen said so and nothing else: no row was
//! written, so a customer who wanted to pay left no trace the operator could act on — and the
//! screen's own failure text pointed at a "support ticket", which this app does not have.
//!
//! The record is the point. `plan_requests` (migration 112) holds one row per ask:
//!
//! * `POST /api/v1/plan-requests`          — the customer's own ask (tenant-scoped, idempotent
//!   while an earlier ask for the same plan is still open or being handled)
//! * `GET  /api/v1/plan-requests`          — this workspace's asks, newest first, with their state
//! * `GET  /api/v1/admin/plan-requests`    — the operator's queue across every workspace
//! * `PUT  /api/v1/admin/plan-requests/:id`— the operator moves the state on
//!
//! The write is real, not a notice: the state the customer reads on reload comes from the row.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// The states an ask can be in. `open` is what a customer files; `contacted` means the operator
/// has reached out; `done`/`declined` are terminal for the customer's screen.
pub const STATUSES: [&str; 4] = ["open", "contacted", "done", "declined"];

#[derive(Deserialize)]
pub struct CreateBody {
    pub plan_slug: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub status: String,
}

fn parse_tenant(auth: &AuthUser) -> AppResult<Uuid> {
    Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))
}

/// `POST /api/v1/plan-requests` — record this workspace's ask for a paid plan.
///
/// Refusals are specific on purpose: an unknown slug is a 404, a $0 plan is a 400 (there is
/// nothing to buy), and a second ask while the first is still open returns the EXISTING row with
/// `status: "existing"` instead of minting a duplicate the operator would have to reconcile.
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = parse_tenant(&auth)?;
    let user_id = Uuid::parse_str(&auth.user_id).ok();
    let slug = body.plan_slug.trim().to_string();
    if slug.is_empty() {
        return Err(AppError::BadRequest("plan_slug is required".into()));
    }

    let plan: Option<(String, f64)> =
        sqlx::query_as("SELECT name, price::float8 FROM plans WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await?;
    let (plan_name, price) =
        plan.ok_or_else(|| AppError::NotFound(format!("No plan with slug '{slug}'")))?;
    if price <= 0.0 {
        return Err(AppError::BadRequest(
            "That plan costs nothing — there is nothing to request.".into(),
        ));
    }

    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM plan_requests
          WHERE tenant_id = $1 AND plan_slug = $2 AND status IN ('open','contacted')
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(id) = existing {
        return Ok((
            StatusCode::OK,
            Json(json!({
                "id": id,
                "status": "existing",
                "message": "Your request is already with our team."
            })),
        ));
    }

    let note: String = body.note.trim().chars().take(1000).collect();
    let row = sqlx::query(
        "INSERT INTO plan_requests (tenant_id, user_id, requested_by, plan_slug, plan_name, note)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, status, created_at::text AS created_at",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&auth.email)
    .bind(&slug)
    .bind(&plan_name)
    .bind(&note)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": row.get::<Uuid, _>("id"),
            "status": row.get::<String, _>("status"),
            "plan_slug": slug,
            "plan_name": plan_name,
            "created_at": row.get::<String, _>("created_at"),
        })),
    ))
}

/// `GET /api/v1/plan-requests` — this workspace's asks, newest first.
pub async fn list_mine(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let tenant_id = parse_tenant(&auth)?;
    let rows = sqlx::query(
        "SELECT id, plan_slug, plan_name, note, status, created_at::text AS created_at
           FROM plan_requests WHERE tenant_id = $1
          ORDER BY created_at DESC LIMIT 50",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let requests: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "plan_slug": r.get::<String, _>("plan_slug"),
                "plan_name": r.get::<String, _>("plan_name"),
                "note": r.get::<String, _>("note"),
                "status": r.get::<String, _>("status"),
                "created_at": r.get::<String, _>("created_at"),
            })
        })
        .collect();

    Ok(Json(json!({ "requests": requests })))
}

/// `GET /api/v1/admin/plan-requests` — the operator's queue. Optional `?status=open`.
pub async fn admin_list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let rows = sqlx::query(
        "SELECT pr.id, pr.plan_slug, pr.plan_name, pr.note, pr.status, pr.requested_by,
                pr.created_at::text AS created_at, t.name AS tenant_name, t.id AS tenant_id
           FROM plan_requests pr
           JOIN tenants t ON t.id = pr.tenant_id
          ORDER BY (pr.status = 'open') DESC, pr.created_at DESC
          LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;

    let requests: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "tenant_id": r.get::<Uuid, _>("tenant_id"),
                "tenant_name": r.get::<String, _>("tenant_name"),
                "requested_by": r.get::<String, _>("requested_by"),
                "plan_slug": r.get::<String, _>("plan_slug"),
                "plan_name": r.get::<String, _>("plan_name"),
                "note": r.get::<String, _>("note"),
                "status": r.get::<String, _>("status"),
                "created_at": r.get::<String, _>("created_at"),
            })
        })
        .collect();

    let open = requests
        .iter()
        .filter(|r| r.get("status").and_then(|s| s.as_str()) == Some("open"))
        .count();

    Ok(Json(json!({ "requests": requests, "open": open })))
}

/// `PUT /api/v1/admin/plan-requests/:id` — move one ask's state on.
pub async fn admin_update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let id = Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid request id".into()))?;
    let status = body.status.trim().to_lowercase();
    if !STATUSES.contains(&status.as_str()) {
        return Err(AppError::BadRequest(format!(
            "status must be one of: {}",
            STATUSES.join(", ")
        )));
    }
    let row = sqlx::query(
        "UPDATE plan_requests SET status = $2, updated_at = NOW()
          WHERE id = $1
         RETURNING id, status, updated_at::text AS updated_at",
    )
    .bind(id)
    .bind(&status)
    .fetch_optional(&state.pool)
    .await?;
    let row = row.ok_or_else(|| AppError::NotFound("No such plan request".into()))?;

    Ok(Json(json!({
        "id": row.get::<Uuid, _>("id"),
        "status": row.get::<String, _>("status"),
        "updated_at": row.get::<String, _>("updated_at"),
    })))
}
