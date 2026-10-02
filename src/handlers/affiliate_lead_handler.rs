// Affiliate lead handler
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

pub async fn submit_affiliate_lead(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Attribution anchor is the authenticated user account. Resolve the affiliate
    // from the user — never trust a body-supplied affiliate_id.
    let affiliate: Option<String> =
        sqlx::query_scalar("SELECT id FROM affiliates WHERE email = $1 AND tenant_id = $2")
            .bind(&auth.email)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    if affiliate.is_none() {
        return Err(AppError::Forbidden(
            "Request affiliate status before submitting leads".into(),
        ));
    }

    // Tag the lead with the referring user account so attribution resolves.
    let created_by = Uuid::parse_str(&auth.user_id).ok();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO leads (id, tenant_id, name, email, phone, source, status, custom_fields, created_by) VALUES ($1, $2, $3, $4, $5, 'affiliate', 'new', $6, $7)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(payload["name"].as_str().unwrap_or(""))
    .bind(payload["email"].as_str().unwrap_or(""))
    .bind(payload["phone"].as_str().unwrap_or(""))
    .bind(&payload["custom_fields"])
    .bind(created_by)
    .execute(&state.pool)
    .await?;

    // Outbound webhook delivery (kanban t_431faa99) — see web_to_lead_handler: every writer of a
    // `leads` row emits `lead.created`, so no capture path is silently outside the subscription.
    crate::webhooks::spawn(
        state.pool.clone(),
        tenant_id,
        crate::webhooks::LEAD_CREATED,
        crate::webhooks::lead_created(
            id,
            payload["name"].as_str(),
            payload["email"].as_str(),
            payload["phone"].as_str(),
            None,
            Some("affiliate"),
            &[],
        ),
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.to_string(), "message": "Lead submitted"})),
    ))
}

pub async fn list_affiliate_prospects(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // leads.created_at is TIMESTAMPTZ; NaiveDateTime made this list 500 as soon as a lead with
    // source='affiliate' existed (none did yet, so the bug was latent).
    let rows: Vec<(Uuid, String, Option<String>, Option<String>, Option<String>, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, name, email, phone, source, COALESCE(status, 'new') AS status, created_at FROM leads WHERE tenant_id = $1 AND source = 'affiliate' ORDER BY created_at DESC LIMIT 50"
    ).bind(tenant_id).fetch_all(&state.pool).await?;
    let leads: Vec<Value> = rows.iter().map(|r| json!({"id": r.0.to_string(), "name": r.1, "email": r.2, "phone": r.3, "source": r.4, "status": r.5, "created_at": r.6})).collect();
    Ok(Json(json!(leads)))
}

pub async fn check_affiliate_for_email(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or(auth.email.as_str());
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT id FROM affiliates WHERE email = $1 AND tenant_id = $2")
            .bind(email)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    Ok(Json(json!({"exists": row.is_some()})))
}

pub async fn log_lead_movement(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let lead_id_str = payload["lead_id"].as_str().unwrap_or("");
    let to_stage = payload["to_stage"].as_str().unwrap_or("");
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    if let Ok(lead_id) = Uuid::parse_str(lead_id_str) {
        sqlx::query(
            "UPDATE leads SET stage = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3",
        )
        .bind(to_stage)
        .bind(lead_id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    }
    Ok(Json(json!({"message": "Movement logged"})))
}

/// What makes a lead part of the affiliate programme — ONE definition, used by both the list and
/// the summary below, so the two can never disagree.
///
/// Three ways in, and all three are real:
///   * `source = 'affiliate'`      — submitted through an affiliate's own form
///   * `created_by IS NOT NULL`    — the capture stamped the referring user account on the lead
///   * it carries a product's SYSTEM tag — the tag is what credits an affiliate, so a tagged lead
///     belongs on this screen even if the stamp is somehow missing
macro_rules! programme_where {
    () => {
        "\
l.source = 'affiliate' \
 OR l.created_by IS NOT NULL \
 OR EXISTS (SELECT 1 FROM tags t \
             WHERE t.is_system = true \
               AND l.tags ? t.name \
               AND EXISTS (SELECT 1 FROM affiliate_products p \
                            WHERE p.system_tag_id = t.id AND p.is_active = true))"
    };
}

/// The list and the count, assembled at COMPILE time from the ONE where-clause above. The widened
/// gate rule 5d (2026-10-02) reports SQL BUILT at run time, and `format!("… WHERE {PROGRAMME_WHERE} …")`
/// was exactly that. Each statement is now a `concat!` of literals, so no statement text exists in a
/// variable — the sanctioned shape, and byte-identical to what the `format!` produced.
const SQL_PROGRAMME_LEADS: &str = concat!(
    "SELECT l.id, l.name, l.email, l.phone, l.source, COALESCE(l.status, 'new') AS status, \
                COALESCE(l.tags, '[]'::jsonb) AS tags, l.created_at, \
                (SELECT a.name FROM affiliates a WHERE a.user_id = l.created_by \
                  ORDER BY a.created_at LIMIT 1) AS referrer_name, \
                (SELECT a.id::text FROM affiliates a WHERE a.user_id = l.created_by \
                  ORDER BY a.created_at LIMIT 1) AS referrer_id, \
                EXISTS (SELECT 1 FROM affiliates a2 WHERE lower(a2.email) = lower(l.email)) AS became_affiliate, \
                (SELECT count(*) FROM affiliate_commissions c WHERE c.lead_id = l.id) AS commission_count, \
                (SELECT COALESCE(sum(c.amount), 0)::float8 FROM affiliate_commissions c WHERE c.lead_id = l.id) AS commission_total, \
                (SELECT c.status FROM affiliate_commissions c WHERE c.lead_id = l.id \
                  ORDER BY c.created_at DESC LIMIT 1) AS commission_status, \
                (SELECT COALESCE(SUM(c.amount), 0)::float8 FROM affiliate_commissions c \
                  WHERE c.lead_id = l.id AND c.status IN ('earned', 'paid')) AS earned, \
                (SELECT p.slug FROM users u \
                   JOIN tenant_plan_subscriptions tps ON tps.tenant_id = u.tenant_id AND tps.status = 'active' \
                   JOIN plans p ON p.id = tps.plan_id \
                  WHERE u.email = l.email ORDER BY tps.start_date DESC LIMIT 1) AS current_plan, \
                (SELECT COALESCE(p.price, 0)::float8 FROM users u \
                   JOIN tenant_plan_subscriptions tps ON tps.tenant_id = u.tenant_id AND tps.status = 'active' \
                   JOIN plans p ON p.id = tps.plan_id \
                  WHERE u.email = l.email ORDER BY tps.start_date DESC LIMIT 1) AS current_price, \
                (SELECT m.movement FROM affiliate_plan_movements m WHERE m.lead_id = l.id \
                  ORDER BY m.occurred_at DESC LIMIT 1) AS last_movement, \
                (SELECT m.occurred_at FROM affiliate_plan_movements m WHERE m.lead_id = l.id \
                  ORDER BY m.occurred_at DESC LIMIT 1) AS last_movement_at \
         FROM leads l WHERE ",
    programme_where!(),
    " ORDER BY l.created_at DESC LIMIT 200"
);

const SQL_PROGRAMME_COUNT: &str = concat!(
    "SELECT count(*) AS total, \
                count(*) FILTER (WHERE l.created_by IS NOT NULL) AS credited, \
                count(*) FILTER (WHERE l.source = 'affiliate') AS from_affiliate_forms, \
                count(*) FILTER (WHERE EXISTS (SELECT 1 FROM affiliates a2 \
                                                WHERE lower(a2.email) = lower(l.email))) AS joined, \
                count(*) FILTER (WHERE EXISTS (SELECT 1 FROM affiliate_plan_movements m \
                                                WHERE m.lead_id = l.id AND m.movement = 'upgrade' \
                                                  AND m.to_price > 0)) AS upgraded, \
                count(*) FILTER (WHERE EXISTS (SELECT 1 FROM affiliate_plan_movements m \
                                                WHERE m.lead_id = l.id AND m.movement = 'downgrade')) AS downgraded \
         FROM leads l WHERE ",
    programme_where!()
);

/// GET /api/v1/admin/affiliate-leads
///
/// David's Affiliate Leads screen (2026-09-28): *"the admin should see all the affiliate leads and
/// which affiliate they belong to. Direct signups show system. And a column for whether the lead
/// themselves became an affiliate."*
///
/// Deliberately PROGRAMME-WIDE, not tenant-scoped: the leads this screen is about belong to the
/// AFFILIATES' tenants (each capture is stored under the tenant whose form took it), so a
/// tenant-scoped read would show the platform admin an empty screen forever — the very bug the
/// old `/api/v1/affiliate/leads` endpoint has for this purpose. Admin only.
pub async fn list_programme_leads(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    // The referrer is resolved from the lead's own `created_by` stamp — that is the attribution
    // anchor, so this can never disagree with who actually gets paid. `became_affiliate` is a
    // DIFFERENT question (did the lead join the programme themselves) and is matched on email.
    let rows = sqlx::query(SQL_PROGRAMME_LEADS)
        .fetch_all(&state.pool)
        .await?;

    let leads: Vec<Value> = rows
        .iter()
        .map(|r| {
            let referrer: Option<String> = r.try_get("referrer_name").ok().flatten();
            json!({
                "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
                "name": r.try_get::<Option<String>, _>("name").ok().flatten(),
                "email": r.try_get::<Option<String>, _>("email").ok().flatten(),
                "phone": r.try_get::<Option<String>, _>("phone").ok().flatten(),
                "source": r.try_get::<Option<String>, _>("source").ok().flatten(),
                "status": r.try_get::<String, _>("status").unwrap_or_else(|_| "new".into()),
                "tags": r.try_get::<Value, _>("tags").unwrap_or_else(|_| json!([])),
                "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").ok(),
                // "system" is the honest answer when nothing refers this lead: nobody gets paid for it.
                "affiliate_name": referrer.clone().unwrap_or_else(|| "system".into()),
                "affiliate_id": r.try_get::<Option<String>, _>("referrer_id").ok().flatten(),
                "attributed": referrer.is_some(),
                "became_affiliate": r.try_get::<bool, _>("became_affiliate").unwrap_or(false),
                "commission_count": r.try_get::<i64, _>("commission_count").unwrap_or(0),
                "commission_total": format!("{:.2}", r.try_get::<f64, _>("commission_total").unwrap_or(0.0)),
                "commission_status": r.try_get::<Option<String>, _>("commission_status").ok().flatten(),
                "earned": format!("{:.2}", r.try_get::<f64, _>("earned").unwrap_or(0.0)),
                "plan": r.try_get::<Option<String>, _>("current_plan").ok().flatten(),
                "pays": r.try_get::<Option<f64>, _>("current_price").ok().flatten().unwrap_or(0.0) > 0.0,
                "last_movement": r.try_get::<Option<String>, _>("last_movement").ok().flatten(),
                "last_movement_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_movement_at").ok().flatten(),
                "last_movement_date": r
                    .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_movement_at")
                    .ok()
                    .flatten()
                    .map(|d| d.format("%Y-%m-%d").to_string()),
            })
        })
        .collect();

    // Counts come from the same predicate, over the whole table — not from the 200-row page, so the
    // headline numbers stay true as the programme grows.
    let c = sqlx::query(SQL_PROGRAMME_COUNT)
        .fetch_one(&state.pool)
        .await?;
    let paid: f64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(c.amount), 0)::float8 FROM affiliate_commissions c \
         JOIN leads l ON l.id = c.lead_id",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0.0);

    Ok(Json(json!({
        "summary": {
            "total": c.try_get::<i64, _>("total").unwrap_or(0),
            "credited": c.try_get::<i64, _>("credited").unwrap_or(0),
            "from_affiliate_forms": c.try_get::<i64, _>("from_affiliate_forms").unwrap_or(0),
            "became_affiliates": c.try_get::<i64, _>("joined").unwrap_or(0),
            "upgraded": c.try_get::<i64, _>("upgraded").unwrap_or(0),
            "downgraded": c.try_get::<i64, _>("downgraded").unwrap_or(0),
            "commission_value": format!("{:.2}", paid),
        },
        "limit": 200,
        "leads": leads
    })))
}
