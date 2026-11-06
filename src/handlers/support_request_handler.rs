//! Support requests — a signed-in workspace ASKS FOR HELP, and reads the answer.
//!
//! WHY THIS MODULE EXISTS (measured 2026-11-06, cron `default`): this app's checkout and plan
//! refusals tell the customer to "contact support", and until now there was nothing to contact —
//! no support surface in the tenant console and no address a signed-in customer could see. The
//! other five apps in the fleet all give a signed-in customer an in-product way to reach the team
//! (`support-tickets` on ADASwift / IncentiveSwift / MissedCall, `tickets` on WorkflowSwift and
//! CoreSwift); this one had none. `plan_request_handler`'s own docs record the same hole.
//!
//! WHAT IS DIFFERENT FROM `plan_requests`: nothing here moves a plan or charges anything. A request
//! is a subject + a message; the operator answers it with a sentence; the customer reads the state
//! back from the same row on reload. That is the whole effect, and it is real (the write is a row,
//! not a notice) — the same "the record is the point" rule the plan-request module established.
//!
//! Routes:
//! * `GET  /api/v1/support-requests`         — this workspace's requests, newest first
//! * `POST /api/v1/support-requests`         — file one (tenant-scoped, idempotent while an earlier
//!   request with the SAME subject is still open, so a double click is one row)
//! * `GET  /api/v1/admin/support-requests`   — the operator's queue across every workspace
//! * `PUT  /api/v1/admin/support-requests/:id` — move the state on and/or write the reply
//!
//! Every route is authenticated. None of them is on the anonymous allowlist, so the fail-closed
//! global auth middleware covers them; the two `/admin/` arms additionally re-check the role.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// The states a request can be in. `open` is what a customer files; `answered` means the operator
/// wrote the reply the customer reads; `closed` ends it without needing one.
pub const STATUSES: [&str; 3] = ["open", "answered", "closed"];

/// The longest subject accepted — enough for a sentence, short enough that it cannot be used as a
/// message body (the database enforces the same bound, so a client cannot bypass it).
const MAX_SUBJECT: usize = 120;
/// The longest message accepted. Matches the column's CHECK.
const MAX_BODY: usize = 4000;

#[derive(Deserialize)]
pub struct CreateBody {
    pub subject: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub status: String,
    /// The sentence the customer reads. Optional so an operator can close a row without writing
    /// one; when it IS sent it lands in the same write as the status, never as a second step.
    #[serde(default)]
    pub reply: Option<String>,
}

fn parse_tenant(auth: &AuthUser) -> AppResult<Uuid> {
    Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))
}

fn trim_cap(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// `POST /api/v1/support-requests` — file a help request for this workspace.
///
/// Refusals are specific: an empty subject or an empty message is a 400 (there would be nothing to
/// answer), and a second request with the SAME subject while the first is still open returns that
/// existing row with `status: "existing"` instead of minting a duplicate the operator would have to
/// reconcile. The tenant always comes from the credential, never from the body.
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = parse_tenant(&auth)?;
    let user_id = Uuid::parse_str(&auth.user_id).ok();

    let subject = trim_cap(&body.subject, MAX_SUBJECT);
    if subject.is_empty() {
        return Err(AppError::BadRequest(
            "Give your request a short subject so the team knows what it is about.".into(),
        ));
    }
    let message = trim_cap(&body.body, MAX_BODY);
    if message.is_empty() {
        return Err(AppError::BadRequest(
            "Write your message so the team knows what you need.".into(),
        ));
    }

    // A double press (or a retry after a flaky network) is ONE request, not two rows. The operator
    // queue reads by row, so a duplicate would be two things to answer.
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM support_requests
          WHERE tenant_id = $1 AND lower(subject) = lower($2) AND status = 'open'
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(&subject)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(id) = existing {
        return Ok((
            StatusCode::OK,
            Json(json!({
                "id": id,
                "status": "existing",
                "message": "You already have an open request with that subject — it is with our team."
            })),
        ));
    }

    let row = sqlx::query(
        "INSERT INTO support_requests (tenant_id, user_id, requested_by, subject, body)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, status, created_at::text AS created_at",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&auth.email)
    .bind(&subject)
    .bind(&message)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": row.get::<Uuid, _>("id"),
            "status": row.get::<String, _>("status"),
            "subject": subject,
            "created_at": row.get::<String, _>("created_at"),
            "message": "Your request is with our team. Its answer appears on this screen.",
        })),
    ))
}

/// `GET /api/v1/support-requests` — this workspace's requests, newest first.
pub async fn list_mine(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let tenant_id = parse_tenant(&auth)?;
    let rows = sqlx::query(
        "SELECT id, subject, body, status, reply, replied_at::text AS replied_at,
                created_at::text AS created_at
           FROM support_requests WHERE tenant_id = $1
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
                "subject": r.get::<String, _>("subject"),
                "body": r.get::<String, _>("body"),
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

/// `GET /api/v1/admin/support-requests` — the operator's queue. Optional `?status=open`.
///
/// Harness-tagged workspaces are held back unless `?include_probe=1`, exactly like
/// `admin_list` for plan requests: a probe's request is not a customer's, and it would otherwise
/// sit in the operator's queue for ever (the teardown retires the identity, not every row it made).
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let status_filter = params
        .get("status")
        .map(|s| s.trim().to_lowercase())
        .filter(|s| STATUSES.contains(&s.as_str()));

    let rows = sqlx::query(
        "SELECT sr.id, sr.subject, sr.body, sr.status, sr.reply,
                sr.replied_at::text AS replied_at, sr.requested_by,
                sr.created_at::text AS created_at,
                t.name AS tenant_name, t.id AS tenant_id
           FROM support_requests sr
           JOIN tenants t ON t.id = sr.tenant_id
          WHERE ($1::text IS NULL OR sr.status = $1::text)
            AND ($2::bool OR COALESCE(t.probe_harness, '') = '')
          ORDER BY (sr.status = 'open') DESC, sr.created_at DESC
          LIMIT 200",
    )
    .bind(status_filter.as_deref())
    .bind(
        params
            .get("include_probe")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
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
                "subject": r.get::<String, _>("subject"),
                "body": r.get::<String, _>("body"),
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

/// `PUT /api/v1/admin/support-requests/:id` — move one request's state on / write its reply.
///
/// The reply and the state travel in ONE write: a customer must never read a new status with the
/// previous explanation still attached. `replied_at` is stamped only when a reply actually lands,
/// so the customer's screen can say "answered" and mean it.
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
    let reply: Option<String> = body
        .reply
        .map(|r| trim_cap(&r, MAX_BODY))
        .filter(|r| !r.is_empty());

    // An `answered` row with no reply would leave the customer reading "answered" and nothing else,
    // which is worse than leaving it open. Require the sentence when that is the state asked for.
    if status == "answered" && reply.is_none() {
        let has_reply: Option<bool> = sqlx::query_scalar(
            "SELECT COALESCE(NULLIF(BTRIM(reply), ''), '') <> '' FROM support_requests WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
        if has_reply != Some(true) {
            return Err(AppError::BadRequest(
                "Write the answer before marking a request answered.".into(),
            ));
        }
    }

    let row = sqlx::query(
        "UPDATE support_requests
            SET status = $2,
                reply = COALESCE($3, reply),
                replied_at = CASE WHEN $3 IS NULL THEN replied_at ELSE NOW() END,
                updated_at = NOW()
          WHERE id = $1
        RETURNING id, tenant_id, subject, requested_by, status, reply,
                  replied_at::text AS replied_at,
                  updated_at::text AS updated_at",
    )
    .bind(id)
    .bind(&status)
    .bind(reply.as_deref())
    .fetch_optional(&state.pool)
    .await?;
    let row = row.ok_or_else(|| AppError::NotFound("No such support request".into()))?;

    // THE ANSWER IS ALSO DELIVERED (cron 2026-11-06). Until now the operator's sentence was stored
    // and the person who asked was told nothing: they had to return to the Support screen and hope
    // to notice. `plan_request_handler` has mailed the asker since 2026-10-19 for exactly this
    // reason, and this module shipped without it the same day it was written.
    //
    // Only a write that CARRIED a sentence mails — marking a row closed (or flipping a status with
    // no new answer) must not post the customer an empty reply. A failure is REPORTED, never
    // thrown: the row is already written, and losing the operator's answer because the mail
    // provider hiccuped would be worse than the missing notification. A reserved/harness recipient
    // is withheld by the send funnel and recorded as a non-send, so a probe is never emailed.
    let tenant_id = row.get::<Uuid, _>("tenant_id");
    let subject_line = row.get::<String, _>("subject");
    let requested_by = row.get::<String, _>("requested_by");
    let mut notified = false;
    let mut notify_detail = String::new();
    if let Some(written) = reply.as_deref() {
        if requested_by.trim().is_empty() {
            notify_detail = "The request has no address to notify.".to_string();
        } else {
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
                .unwrap_or_else(|| "there".to_string());
            let status_word = match status.as_str() {
                "closed" => "closed",
                "answered" => "answered",
                _ => "open",
            };
            // A reserved/harness recipient is a NON-SEND: the funnel withholds it (no provider
            // request) and records `withheld`. Reporting that as "the customer was emailed" is the
            // exact misreport this fleet has retracted twice, so ask the same predicate the funnel
            // uses and keep `notified` false when it fires.
            let reserved = crate::security::probe_addr::harness_domain(&requested_by).is_some();
            match crate::email::send_support_reply_email(
                &state.pool,
                tenant_id,
                &requested_by,
                &greeting,
                &subject_line,
                status_word,
                written,
            )
            .await
            {
                Ok(()) if reserved => {
                    notify_detail =
                        "the requesting address is a reserved/harness address, so the notice was \
                         withheld and no email was sent"
                            .to_string();
                }
                Ok(()) => notified = true,
                Err(e) => {
                    tracing::warn!(
                        request = %id,
                        to = %requested_by,
                        "support-reply mail was not sent: {e}"
                    );
                    notify_detail = e;
                }
            }
        }
    }

    Ok(Json(json!({
        "id": row.get::<Uuid, _>("id"),
        "status": row.get::<String, _>("status"),
        "reply": row.get::<String, _>("reply"),
        "replied_at": row.get::<Option<String>, _>("replied_at"),
        "updated_at": row.get::<String, _>("updated_at"),
        // Whether the customer was actually TOLD. The operator's screen says which, so a silent
        // mail failure never looks like a delivered answer.
        "notified": notified,
        "notify_detail": notify_detail,
    })))
}
