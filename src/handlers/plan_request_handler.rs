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
    /// An optional sentence for the customer, stored WITH the state change so the Upgrade screen
    /// can explain a `declined`/`contacted` chip instead of leaving the customer guessing.
    #[serde(default)]
    pub reply: Option<String>,
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
        "SELECT id, plan_slug, plan_name, note, status, reply, replied_at::text AS replied_at,
                created_at::text AS created_at
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
                "reply": r.get::<String, _>("reply"),
                "replied_at": r.get::<Option<String>, _>("replied_at"),
                "created_at": r.get::<String, _>("created_at"),
            })
        })
        .collect();

    Ok(Json(json!({ "requests": requests })))
}

/// `DELETE /api/v1/plan-requests/:id` — this workspace WITHDRAWS its own open ask.
///
/// WHY (measured 2026-10-19): a customer could file an ask and then had no way to take it back. The
/// row IS the record the operator works from — a `done` answer MOVES the workspace onto that plan —
/// so an ask that is no longer wanted sits in the queue and can be acted on after the customer
/// changed their mind. Their own screen lists the ask, so the control belongs beside it.
///
/// Scope and refusals, deliberately narrow:
/// * the row must belong to the CALLER'S OWN tenant (`tenant_id = $2`), so an id from another
///   workspace answers **404** — the same answer as an id that does not exist, which confirms
///   nothing about whether such a row is anywhere in the database;
/// * only `open` / `contacted` can be withdrawn. An ANSWERED ask (`done` / `declined`) is a decision
///   the workspace already received, so that write answers **409** with its reason instead of
///   quietly rewriting history;
/// * it is idempotent for this workspace: re-sending it for an already-withdrawn ask answers 200
///   with `withdrawn: false`, because the state the caller asked for is already true;
/// * nothing moves and nothing is charged — no plan, no subscription, no commission. The row's
///   `status` is the whole effect, and it is what the operator's queue reads.
pub async fn withdraw(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = parse_tenant(&auth)?;
    let id = Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid request id".into()))?;

    let updated: Option<(Uuid, String, String)> = sqlx::query_as(
        "UPDATE plan_requests
            SET status = 'withdrawn', updated_at = NOW()
          WHERE id = $1 AND tenant_id = $2 AND status IN ('open','contacted')
      RETURNING id, plan_slug, plan_name",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some((id, plan_slug, plan_name)) = updated {
        return Ok((
            StatusCode::OK,
            Json(json!({
                "id": id,
                "status": "withdrawn",
                "withdrawn": true,
                "plan_slug": plan_slug,
                "plan_name": plan_name,
            })),
        ));
    }

    // No row was open for this workspace. Say WHICH of the three states it is, from the row itself
    // (this workspace's only): absent -> 404, already withdrawn -> 200, answered -> 409.
    let current: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, status, plan_name FROM plan_requests WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    match current {
        None => Err(AppError::NotFound("No such plan request".into())),
        Some((id, status, plan_name)) if status == "withdrawn" => Ok((
            StatusCode::OK,
            Json(json!({
                "id": id,
                "status": "withdrawn",
                "withdrawn": false,
                "plan_name": plan_name,
            })),
        )),
        Some((id, status, plan_name)) => Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "id": id,
                "status": status,
                "withdrawn": false,
                "plan_name": plan_name,
                "message": "That request has already been answered, so it cannot be withdrawn. \
                            Reply to the note on it if the answer is not what you wanted."
            })),
        )),
    }
}

/// `GET /api/v1/admin/plan-requests` — the operator's queue. Optional `?status=open`.
pub async fn admin_list(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let rows = sqlx::query(
        "SELECT pr.id, pr.plan_slug, pr.plan_name, pr.note, pr.status, pr.reply,
                pr.replied_at::text AS replied_at, pr.requested_by,
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
                "reply": r.get::<String, _>("reply"),
                "replied_at": r.get::<Option<String>, _>("replied_at"),
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
    // A WITHDRAWN ask is closed BY THE CUSTOMER: it can no longer be granted, declined or moved.
    // Without this guard the operator's queue still offered "done" on that row (the buttons only
    // exclude the state the row already carries), so answering it would have moved the workspace onto
    // a plan its owner had explicitly taken back — the exact outcome the withdraw control exists to
    // prevent. Reads the row by id alone because the operator's queue is install-wide.
    if status != "withdrawn" {
        let current: Option<String> =
            sqlx::query_scalar("SELECT status FROM plan_requests WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.pool)
                .await?;
        if current.as_deref() == Some("withdrawn") {
            return Err(AppError::Conflict(
                "The customer withdrew this request, so it cannot be answered. \
                 They can file a new one at any time."
                    .into(),
            ));
        }
    }
    // The note is optional, but when it IS sent it belongs to the same write as the state: a
    // customer must never read a new status with the previous explanation still attached.
    let reply: Option<String> = body
        .reply
        .map(|r| r.trim().chars().take(1000).collect::<String>())
        .filter(|r| !r.is_empty());
    // The row BEFORE the write travels back with it, so "did anything the customer can read actually
    // change?" is answered by the database rather than by trusting the request body. Saving a status
    // the row already carries must not mail the customer a second time.
    let row = sqlx::query(
        "WITH old AS (
            SELECT pr.status AS old_status, pr.reply AS old_reply, pr.plan_name,
                   pr.plan_slug, pr.requested_by, pr.tenant_id
              FROM plan_requests pr WHERE pr.id = $1
         ), upd AS (
            UPDATE plan_requests
               SET status = $2,
                   reply = COALESCE($3, reply),
                   replied_at = CASE WHEN $3 IS NULL THEN replied_at ELSE NOW() END,
                   updated_at = NOW()
             WHERE id = $1
         RETURNING id, status, reply, replied_at::text AS replied_at,
                   updated_at::text AS updated_at
         )
         SELECT upd.*, old.old_status, old.old_reply, old.plan_name, old.plan_slug,
                old.requested_by, old.tenant_id, t.name AS tenant_name
           FROM upd CROSS JOIN old LEFT JOIN tenants t ON t.id = old.tenant_id",
    )
    .bind(id)
    .bind(&status)
    .bind(reply.as_deref())
    .fetch_optional(&state.pool)
    .await?;
    let row = row.ok_or_else(|| AppError::NotFound("No such plan request".into()))?;

    let new_status = row.get::<String, _>("status");
    let new_reply = row.get::<String, _>("reply");
    let old_status = row.get::<String, _>("old_status");
    let old_reply = row.get::<String, _>("old_reply");
    let plan_name = row.get::<String, _>("plan_name");
    let requested_by = row.get::<String, _>("requested_by");
    let tenant_id = row.get::<Uuid, _>("tenant_id");
    let tenant_name = row.try_get::<String, _>("tenant_name").unwrap_or_default();
    let plan_slug = row.get::<String, _>("plan_slug");

    // CLOSING THE LOOP (2026-10-19): `done` is the operator saying the workspace GOT the plan.
    // Until this arm existed the status moved and the workspace's plan did not: the ask read
    // "complete", the customer was mailed "complete", and the tier on their dashboard was still the
    // old one, because moving them was a separate action on the Tenants screen that nothing asked
    // for. The write is `plan_handler::set_active_plan` — the only writer of a tenant's plan, the
    // same call the Tenants picker makes — so the old row is deactivated, the movement is dated and
    // a referrer is settled exactly once, by the one code path that knows how.
    //
    // Only on the TRANSITION into `done`, and never when the workspace already holds that plan: a
    // second save must not mint a second subscription row or settle a commission twice. A plan that
    // cannot be resolved does not fail the operator's save — the state change is the record — but it
    // is reported back so the screen can say the plan did not move, instead of implying it did.
    let mut plan_applied = false;
    let mut plan_note: Option<String> = None;
    if new_status == "done" && old_status != "done" {
        let target: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id, slug FROM plans WHERE slug = $1")
                .bind(&plan_slug)
                .fetch_optional(&state.pool)
                .await?;
        match target {
            None => {
                tracing::warn!(
                    request = %id, plan = %plan_slug,
                    "plan-request marked done, but no plan carries that slug — the plan did NOT move"
                );
                plan_note = Some(format!("no plan is called '{plan_slug}' any more"));
            }
            Some((plan_id, slug)) => {
                let already = crate::plan_movement::current_plan(&state.pool, tenant_id)
                    .await
                    .map(|(s, _)| s == slug)
                    .unwrap_or(false);
                if already {
                    plan_note = Some(format!("already on {slug}"));
                } else {
                    match crate::handlers::plan_handler::set_active_plan(
                        &state.pool,
                        tenant_id,
                        plan_id,
                    )
                    .await
                    {
                        Ok((_, applied)) => {
                            plan_applied = true;
                            plan_note = Some(applied);
                            tracing::info!(
                                request = %id, tenant = %tenant_id, plan = %plan_slug,
                                "plan-request done: the workspace was moved onto the plan it asked for"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                request = %id, tenant = %tenant_id, plan = %plan_slug,
                                "plan-request done, but the plan move failed: {e}"
                            );
                            plan_note = Some(format!("the plan move failed: {e}"));
                        }
                    }
                }
            }
        }
    }

    // Tell the customer, but only when their screen would show something new. A mail failure never
    // fails the operator's save: the row is already written, and the sender records a refused send in
    // `email_send_log` (and a reserved/harness recipient as a non-send), where the operator can see it.
    if new_status != old_status || (!new_reply.is_empty() && new_reply != old_reply) {
        let filer_name: Option<String> = sqlx::query_scalar::<_, Option<String>>(
            "SELECT COALESCE(NULLIF(first_name, ''), NULLIF(name, '')) FROM users
              WHERE tenant_id = $1 AND lower(email) = lower($2) LIMIT 1",
        )
        .bind(tenant_id)
        .bind(&requested_by)
        .fetch_optional(&state.pool)
        .await?
        .flatten();
        let greeting = filer_name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                let t = tenant_name.trim().to_string();
                if t.is_empty() {
                    "there".to_string()
                } else {
                    t
                }
            });
        let status_word = match new_status.as_str() {
            "contacted" => "with our team - someone will be in touch",
            "done" => "complete",
            "declined" => "closed",
            _ => "received",
        };
        if let Err(e) = crate::email::send_plan_request_update_email(
            &state.pool,
            tenant_id,
            &requested_by,
            &greeting,
            &plan_name,
            status_word,
            &new_reply,
        )
        .await
        {
            tracing::warn!(
                request = %id,
                to = %requested_by,
                "plan-request update mail was not sent: {e}"
            );
        }
    }

    Ok(Json(json!({
        "id": row.get::<Uuid, _>("id"),
        "status": row.get::<String, _>("status"),
        "reply": row.get::<String, _>("reply"),
        "replied_at": row.get::<Option<String>, _>("replied_at"),
        "updated_at": row.get::<String, _>("updated_at"),
        // Whether the workspace's plan actually moved on this save, and why not when it did not.
        "plan_applied": plan_applied,
        "plan_note": plan_note,
    })))
}
