//! `invoice_handler` — the operator's invoice ledger (2026-11-23).
//!
//! WHY THIS MODULE EXISTS: card payments are OFF on this installation (`payment_providers` holds
//! zero rows), so a plan REQUEST is the buying path and `admin/billing-settings` holds the words
//! that tell a customer how to send money. Nothing stored what a customer OWED or had PAID: the
//! operator's book was his own memory and an inbox, and a customer who had been billed had nothing
//! to point at. ADASwift closed this with its own `invoices` table (migration 036); this is the
//! FunnelSwift side, so a workspace that asks to buy can be billed TODAY with no provider, no key
//! and no card.
//!
//! Four doors, two audiences:
//!   * `GET  /api/v1/invoices`             — the caller's OWN invoices (tenant from the token).
//!   * `GET  /api/v1/admin/invoices`       — the operator's whole book, optionally `?tenant_id=`.
//!   * `POST /api/v1/admin/invoices`       — raise one.
//!   * `PUT  /api/v1/admin/invoices/:id`   — mark paid / void / reopen.
//!
//! The admin doors sit under `/api/v1/admin/`, which `auth::global_auth` gates to a platform
//! operator; each handler ALSO states that predicate itself, like every other admin writer here,
//! so a missing middleware shows up as a test failure rather than an open door. The tenant door is
//! scoped by `auth.tenant_id`, so one workspace can never read another's rows.
//!
//! The amount is deliberately not editable on the update door: an invoice that has been sent can be
//! settled or cancelled, never silently re-priced.
//!
//! A PAID INVOICE FOR A PLAN SEATS THE WORKSPACE ON THAT PLAN (2026-11-23). The tier moves through
//! the one plan mover (`plan_handler::set_active_plan`) on the TRANSITION into `paid` only, never
//! downward, and the customer is told through `notify_plan_change` — whose answer is reported back so
//! the screen can say whether a notice actually went. Until that arm existed the money and the
//! entitlement were two unrelated facts: the operator billed, the customer paid, the bill was marked
//! paid, and the customer's dashboard still showed the old plan.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// `invoices.amount` is NUMERIC(10,2); psql/sqlx hand it back as a decimal that serialises poorly
/// into JSON, so every read casts it to `::text` — the same convention the plan catalogue uses for
/// its price. The console formats it, and a bill never prints a float tail it does not need.
///
/// Every query in this module is a PARAMETERISED LITERAL on purpose: the fleet gate refuses SQL
/// built with `format!`/concatenation, and there is no dynamic identifier here that would need it.
fn row_to_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id").to_string(),
        "tenant_id": r.get::<Uuid, _>("tenant_id").to_string(),
        "tenant_name": r.try_get::<Option<String>, _>("tenant_name").unwrap_or(None),
        "plan_id": r.try_get::<Option<Uuid>, _>("plan_id").ok().flatten().map(|u| u.to_string()),
        "plan_name": r.try_get::<Option<String>, _>("plan_name").unwrap_or(None),
        "amount": r.try_get::<Option<String>, _>("amount").unwrap_or(None),
        "status": r.get::<String, _>("status"),
        "note": r.try_get::<Option<String>, _>("note").unwrap_or(None),
        "due_date": r.try_get::<Option<String>, _>("due_date").unwrap_or(None),
        "paid_at": r.try_get::<Option<String>, _>("paid_at").unwrap_or(None),
        "created_at": r.try_get::<Option<String>, _>("created_at").unwrap_or(None),
    })
}

/// The statuses an invoice can hold. `pending` is what a raise writes.
pub const STATUSES: [&str; 3] = ["pending", "paid", "void"];

fn require_admin(auth: &AuthUser) -> AppResult<()> {
    if auth.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin access required".into()))
    }
}

fn tenant_of(auth: &AuthUser) -> AppResult<Uuid> {
    Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))
}

/// `GET /api/v1/invoices` — this workspace's own invoices, newest first.
///
/// An empty list is a real answer (nothing has been billed), so this never 404s: a customer screen
/// that said "not found" when the book was empty would read as a broken page.
pub async fn list_mine(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let tenant = tenant_of(&auth)?;
    let rows = sqlx::query(
        "SELECT i.id, i.tenant_id, t.name AS tenant_name, i.plan_id, \
                p.name AS plan_name, i.amount::text AS amount, i.status, i.note, \
                i.due_date::text AS due_date, i.paid_at::text AS paid_at, \
                i.created_at::text AS created_at \
           FROM invoices i \
           LEFT JOIN plans p ON p.id = i.plan_id \
           LEFT JOIN tenants t ON t.id = i.tenant_id WHERE i.tenant_id = $1 ORDER BY i.created_at DESC LIMIT 200",
    )
    .bind(tenant)
    .fetch_all(&state.db)
    .await?;
    let items: Vec<Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(
        json!({ "invoices": &items, "items": &items, "count": items.len() }),
    ))
}

/// `GET /api/v1/admin/invoices[?tenant_id=…]` — the operator's whole book.
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    let filter = params
        .get("tenant_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let rows = match filter {
        Some(t) => {
            let tenant = Uuid::parse_str(&t)
                .map_err(|_| AppError::BadRequest("That workspace id is not valid".into()))?;
            sqlx::query(
                "SELECT i.id, i.tenant_id, t.name AS tenant_name, i.plan_id, \
                p.name AS plan_name, i.amount::text AS amount, i.status, i.note, \
                i.due_date::text AS due_date, i.paid_at::text AS paid_at, \
                i.created_at::text AS created_at \
           FROM invoices i \
           LEFT JOIN plans p ON p.id = i.plan_id \
           LEFT JOIN tenants t ON t.id = i.tenant_id WHERE i.tenant_id = $1 ORDER BY i.created_at DESC LIMIT 500",
            )
            .bind(tenant)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query(
                "SELECT i.id, i.tenant_id, t.name AS tenant_name, i.plan_id, \
                p.name AS plan_name, i.amount::text AS amount, i.status, i.note, \
                i.due_date::text AS due_date, i.paid_at::text AS paid_at, \
                i.created_at::text AS created_at \
           FROM invoices i \
           LEFT JOIN plans p ON p.id = i.plan_id \
           LEFT JOIN tenants t ON t.id = i.tenant_id ORDER BY i.created_at DESC LIMIT 500",
            )
            .fetch_all(&state.db)
            .await?
        }
    };
    let items: Vec<Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(
        json!({ "invoices": &items, "items": &items, "count": items.len() }),
    ))
}

#[derive(Deserialize)]
pub struct InvoiceBody {
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    /// "49", 49, "49.00" or "$49.00" — a price a person typed.
    #[serde(default)]
    pub amount: Option<Value>,
    /// "YYYY-MM-DD" from the console's date input, or empty for no due date.
    #[serde(default)]
    pub due_date: Option<Value>,
    /// A sentence for the customer (what the invoice covers).
    #[serde(default)]
    pub note: Option<String>,
    /// pending | paid | void. Absent means pending.
    #[serde(default)]
    pub status: Option<String>,
}

/// A price as a person types it, normalised to two decimals. `None` is a refusal, never a zero —
/// an invoice for nothing is not a bill.
fn money_text(v: &Value) -> Option<String> {
    let raw = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let cleaned = raw.trim_start_matches('$').replace(',', "");
    let parsed: f64 = cleaned.parse().ok()?;
    if !parsed.is_finite() || parsed <= 0.0 || parsed > 1_000_000.0 {
        return None;
    }
    Some(format!("{parsed:.2}"))
}

/// A due date as the console sends it; anything that is not a calendar day is refused, so a typo
/// can never land in the column that decides when a customer is overdue.
fn due_text(v: &Value) -> Option<Option<String>> {
    let raw = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Null => return Some(None),
        _ => return None,
    };
    if raw.is_empty() {
        return Some(None);
    }
    let day = raw.get(..10).unwrap_or(&raw);
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    Some(Some(day.to_string()))
}

fn status_word(v: Option<&String>) -> Option<&'static str> {
    match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("pending") => Some("pending"),
        Some("paid") => Some("paid"),
        Some("void") => Some("void"),
        _ => None,
    }
}

/// `POST /api/v1/admin/invoices` — raise an invoice for one workspace.
pub async fn admin_create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<InvoiceBody>,
) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    let tenant = body
        .tenant_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("Choose the workspace this invoice is for".into()))?;
    let tenant = Uuid::parse_str(tenant)
        .map_err(|_| AppError::BadRequest("That workspace id is not valid".into()))?;
    // A plan is optional: a setup fee or a one-off is still money the operator must be able to
    // record, and refusing it would push that bill back into the inbox this table exists to replace.
    let plan: Option<Uuid> = match body
        .plan_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(p) => Some(
            Uuid::parse_str(p)
                .map_err(|_| AppError::BadRequest("That plan id is not valid".into()))?,
        ),
        None => None,
    };
    let amount = body.amount.as_ref().and_then(money_text).ok_or_else(|| {
        AppError::BadRequest("Enter an amount above zero, for example 49.00".into())
    })?;
    let due = match body.due_date.as_ref() {
        None => None,
        Some(v) => due_text(v)
            .ok_or_else(|| AppError::BadRequest("Enter the due date as a calendar day".into()))?,
    };
    let status = status_word(body.status.as_ref())
        .ok_or_else(|| AppError::BadRequest("A status is pending, paid or void".into()))?;
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(500).collect::<String>());

    let tenant_exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_optional(&state.db)
        .await?;
    if tenant_exists.is_none() {
        return Err(AppError::NotFound("That workspace no longer exists".into()));
    }
    if let Some(p) = plan {
        let plan_exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM plans WHERE id = $1")
            .bind(p)
            .fetch_optional(&state.db)
            .await?;
        if plan_exists.is_none() {
            return Err(AppError::NotFound("That plan no longer exists".into()));
        }
    }

    let row = sqlx::query(
        "WITH ins AS ( \
            INSERT INTO invoices (tenant_id, plan_id, amount, status, due_date, note, paid_at) \
            VALUES ($1, $2, $3::numeric, $4, CAST($5 AS timestamptz), $6, \
                    CASE WHEN $4 = 'paid' THEN now() ELSE NULL END) \
            RETURNING * ) \
         SELECT i.id, i.tenant_id, t.name AS tenant_name, i.plan_id, \
                p.name AS plan_name, i.amount::text AS amount, i.status, i.note, \
                i.due_date::text AS due_date, i.paid_at::text AS paid_at, \
                i.created_at::text AS created_at \
           FROM ins i \
           LEFT JOIN plans p ON p.id = i.plan_id \
           LEFT JOIN tenants t ON t.id = i.tenant_id",
    )
    .bind(tenant)
    .bind(plan)
    .bind(&amount)
    .bind(status)
    .bind(due.as_deref())
    .bind(note.as_deref())
    .fetch_one(&state.db)
    .await?;

    let item = row_to_json(&row);
    Ok(Json(json!({ "item": &item, "invoice": &item })))
}

/// `PUT /api/v1/admin/invoices/:id` — mark paid (stamps `paid_at`), void it (clears the stamp), or
/// put it back to pending.
pub async fn admin_update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<InvoiceBody>,
) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| AppError::NotFound("That invoice no longer exists".into()))?;
    let status = status_word(body.status.as_ref())
        .ok_or_else(|| AppError::BadRequest("A status is pending, paid or void".into()))?;

    // READ THE ROW'S LIFE BEFORE THE WRITE (2026-11-23): the plan is applied only on the
    // TRANSITION into `paid`, so the tier this invoice named and the status it held a moment ago
    // decide whether the money loop closes. A repeat save of an already-paid bill must not mint a
    // second subscription row or settle a referrer's commission twice.
    let before: Option<(Uuid, Option<Uuid>, String)> =
        sqlx::query_as("SELECT tenant_id, plan_id, status FROM invoices WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let Some((tenant_id, plan_id, old_status)) = before else {
        return Err(AppError::NotFound("That invoice no longer exists".into()));
    };

    // The join that names the plan/workspace has to be re-applied after the UPDATE, so the CTE
    // returns the row and the outer SELECT re-joins it (the same shape ADASwift's ledger uses).
    let row = sqlx::query(
        "WITH upd AS ( \
            UPDATE invoices SET status = $2, \
                   paid_at = CASE WHEN $2 = 'paid' THEN COALESCE(paid_at, now()) ELSE NULL END \
             WHERE id = $1 \
            RETURNING * ) \
         SELECT i.id, i.tenant_id, t.name AS tenant_name, i.plan_id, \
                p.name AS plan_name, i.amount::text AS amount, i.status, i.note, \
                i.due_date::text AS due_date, i.paid_at::text AS paid_at, \
                i.created_at::text AS created_at \
           FROM upd i \
           LEFT JOIN plans p ON p.id = i.plan_id \
           LEFT JOIN tenants t ON t.id = i.tenant_id",
    )
    .bind(id)
    .bind(status)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        return Err(AppError::NotFound("That invoice no longer exists".into()));
    };
    let item = row_to_json(&row);

    // CLOSING THE MONEY LOOP (2026-11-23). Until this arm a PAID invoice moved no tier: the
    // operator raised the bill, the customer paid it, the operator marked it paid, and the
    // workspace's plan stayed exactly where it was — money taken and nothing delivered, with the
    // customer's own dashboard still showing the old plan. Payments on this installation are given
    // BY HAND (it holds no payment provider), so this ledger is the till; a till that does not
    // deliver the thing it billed for is a half-fix.
    //
    // The write is the ONE plan mover — `plan_handler::set_active_plan`, the same call the Tenants
    // picker and the plan-request `done` arm make — so the previous subscription is deactivated, the
    // movement is dated and a referrer is settled exactly once. The customer is then told through
    // `notify_plan_change`, whose answer says plainly whether a mail actually went
    // (`emailed` | `unchanged` | `no_recipient` | `withheld` | `mail_failed`) instead of assuming.
    //
    // Three guards, each deliberate:
    //  * only on the TRANSITION into `paid` (read above), so a second save is not a second sale;
    //  * NEVER DOWNWARD — a stale invoice for a cheaper tier cannot demote a workspace that has since
    //    moved higher; tiers are compared on the catalogue price, and an equal-price move still goes
    //    through because that is a product-side switch, not a downgrade;
    //  * a plan that has since been deleted does not fail the operator's save — the status change is
    //    the record — but the sentence comes back so the screen can say the tier did not move.
    let mut plan_applied = false;
    let mut plan_note: Option<String> = None;
    let mut notify_reason: Option<&'static str> = None;
    if status == "paid" && old_status != "paid" {
        if let Some(plan_id) = plan_id {
            let target: Option<(String, f64)> =
                sqlx::query_as("SELECT slug, COALESCE(price, 0)::float8 FROM plans WHERE id = $1")
                    .bind(plan_id)
                    .fetch_optional(&state.db)
                    .await?;
            match target {
                None => plan_note = Some("the plan on this invoice no longer exists".to_string()),
                Some((slug, asked_price)) => {
                    let current = crate::plan_movement::current_plan(&state.db, tenant_id).await;
                    let current_slug = current.as_ref().map(|(s, _)| s.clone());
                    let current_price = current.as_ref().map(|(_, p)| *p).unwrap_or(0.0);
                    if current_slug.as_deref() == Some(slug.as_str()) {
                        plan_note = Some(format!("the workspace is already on {slug}"));
                    } else if current_slug.is_some() && current_price > asked_price + 0.001 {
                        tracing::info!(
                            invoice = %id, tenant = %tenant_id, held = ?current_slug, asked = %slug,
                            "invoice paid: the workspace already holds a dearer plan - no change applied"
                        );
                        plan_note = Some(format!(
                            "the workspace already holds a dearer plan, so it was left as it is (this invoice was for {slug})"
                        ));
                    } else {
                        match crate::handlers::plan_handler::set_active_plan(
                            &state.db, tenant_id, plan_id, None,
                        )
                        .await
                        {
                            Ok((_, moved_to)) => {
                                plan_applied = true;
                                let notice = crate::handlers::plan_handler::notify_plan_change(
                                    &state.db,
                                    tenant_id,
                                    plan_id,
                                    current_slug.as_deref(),
                                )
                                .await;
                                notify_reason = Some(notice.reason);
                                let said = match notice.reason {
                                    "emailed" => "the customer has been emailed",
                                    "withheld" => {
                                        "no notice was sent — the customer's address is on the do-not-mail list"
                                    }
                                    "no_recipient" => "no notice was sent — the workspace has no address",
                                    "mail_failed" => "the notice mail failed to send",
                                    _ => "no notice was sent",
                                };
                                plan_note =
                                    Some(format!("the workspace is now on {moved_to} — {said}"));
                            }
                            Err(e) => {
                                tracing::warn!(
                                    invoice = %id, tenant = %tenant_id, error = %e,
                                    "invoice marked paid but the plan did not move"
                                );
                                plan_note =
                                    Some("the plan could not be moved — see the log".to_string());
                            }
                        }
                    }
                }
            }
        } else {
            plan_note = Some("this invoice names no plan, so no tier was changed".to_string());
        }
    }

    Ok(Json(json!({
        "item": &item,
        "invoice": &item,
        "plan_applied": plan_applied,
        "plan_note": plan_note,
        "notify_reason": notify_reason,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_accepts_what_a_person_types_and_refuses_zero() {
        assert_eq!(money_text(&json!("49")), Some("49.00".to_string()));
        assert_eq!(money_text(&json!("$1,299.5")), Some("1299.50".to_string()));
        assert_eq!(money_text(&json!(12)), Some("12.00".to_string()));
        assert_eq!(money_text(&json!("0")), None);
        assert_eq!(money_text(&json!("-5")), None);
        assert_eq!(money_text(&json!("abc")), None);
    }

    #[test]
    fn due_text_refuses_a_non_day() {
        assert_eq!(
            due_text(&json!("2026-12-01")),
            Some(Some("2026-12-01".into()))
        );
        assert_eq!(due_text(&json!("")), Some(None));
        assert_eq!(due_text(&json!("next friday")), None);
    }

    #[test]
    fn status_word_is_a_closed_vocabulary() {
        assert_eq!(status_word(None), Some("pending"));
        assert_eq!(status_word(Some(&"PAID".to_string())), Some("paid"));
        assert_eq!(status_word(Some(&"void".to_string())), Some("void"));
        assert_eq!(status_word(Some(&"refunded".to_string())), None);
    }
}
