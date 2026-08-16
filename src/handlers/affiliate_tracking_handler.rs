use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub async fn list_affiliate_links(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows: Vec<(Uuid, String, Option<String>, String, Option<f64>, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT al.id, al.affiliate_id, al.target_software, al.tracking_code, al.commission_rate, al.created_at FROM affiliate_links al JOIN affiliates a ON a.id = al.affiliate_id WHERE a.tenant_id = $1 ORDER BY al.created_at DESC"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    let links: Vec<Value> = rows
        .iter()
        .map(|r| json!({"id": r.0.to_string(), "affiliate_id": r.1, "tracking_code": r.3}))
        .collect();
    Ok(Json(json!(links)))
}
pub async fn create_affiliate_link(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // Resolve the affiliate from the authenticated user — never trust a body affiliate_id.
    let (affiliate_id, commission_rate): (String, Option<f64>) = sqlx::query_as(
        "SELECT id, commission_rate FROM affiliates WHERE email = $1 AND tenant_id = $2",
    )
    .bind(&auth.email)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate account not found".into()))?;

    let target_software = payload["target_software"].as_str().unwrap_or("");
    let id = Uuid::new_v4();
    let code = format!("TRACK{}", &id.to_string().replace("-", "")[..12]);
    sqlx::query("INSERT INTO affiliate_links (id, affiliate_id, target_software, tracking_code, commission_rate) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(&affiliate_id)
        .bind(target_software)
        .bind(&code)
        .bind(commission_rate)
        .execute(&state.pool)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": id.to_string(), "tracking_code": code, "commission_rate": commission_rate}),
        ),
    ))
}
pub async fn get_affiliate_stats(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"total_clicks": 0, "total_conversions": 0})))
}
pub async fn list_conversions(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows: Vec<(Uuid, Option<Uuid>, Option<String>, Option<Uuid>, Option<f64>, Option<f64>, Option<String>, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT ac.id, ac.click_id, ac.affiliate_user_id, ac.customer_id, ac.amount, ac.commission, ac.status, ac.converted_at FROM affiliate_conversions ac JOIN affiliate_users au ON au.user_id = ac.affiliate_user_id WHERE au.tenant_id = $1 ORDER BY ac.converted_at DESC LIMIT 50"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    let items: Vec<Value> = rows
        .iter()
        .map(|r| json!({"id": r.0.to_string(), "amount": r.4, "commission": r.5, "status": r.6}))
        .collect();
    Ok(Json(json!(items)))
}
pub async fn track_conversion(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // Resolve the affiliate from the authenticated user (their plan-derived rate is used).
    let (affiliate_id, rate): (String, Option<f64>) = sqlx::query_as(
        "SELECT id, commission_rate FROM affiliates WHERE email = $1 AND tenant_id = $2",
    )
    .bind(&auth.email)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Affiliate account not found".into()))?;

    let amount = payload["amount"].as_f64().unwrap_or(0.0);
    let commission = amount * rate.unwrap_or(0.0) / 100.0;
    let lead_id = payload["lead_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok());

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO affiliate_commissions (id, affiliate_id, lead_id, amount, status) VALUES ($1, $2, $3, $4, 'pending')",
    )
    .bind(id)
    .bind(&affiliate_id)
    .bind(lead_id)
    .bind(commission)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({
        "id": id.to_string(),
        "affiliate_id": affiliate_id,
        "sale_amount": amount,
        "commission": commission,
        "status": "pending",
    })))
}
pub async fn track_click(
    Query(_params): Query<HashMap<String, String>>,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"tracked": true})))
}

/// Cross-app affiliate upgrade event (internal, x-internal-key).
///
/// Called by the other Swift apps (WorkflowSwift, CoreSwift, IncentiveSwift, etc.) when a
/// user — who was originally a FunnelSwift lead provisioned a free account via a system
/// tag — upgrades to a paid plan. Credits the referring affiliate PERMANENTLY: there is no
/// cookie/attribution expiry, so every upgrade (today, a year from now, after a downgrade
/// and re-upgrade) credits the same affiliate, resolved via the lead's originating user
/// account (leads.created_by -> affiliates.user_id).
pub async fn handle_affiliate_upgrade_event(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let key = headers
        .get("x-internal-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if key != state.internal_sync_key {
        return Err(AppError::Forbidden("Invalid internal key".into()));
    }

    let email = payload["email"].as_str().unwrap_or("");
    let source_app = payload["source_app"].as_str().unwrap_or("");
    let plan_name = payload["plan_name"].as_str().unwrap_or("");
    let plan_price = payload["plan_price"].as_f64().unwrap_or(0.0);
    let event_id = payload["event_id"].as_str().map(|s| s.to_string());

    if email.is_empty() || source_app.is_empty() {
        return Err(AppError::BadRequest(
            "email and source_app are required".into(),
        ));
    }

    // Idempotency: skip if this exact upgrade event was already credited.
    if let Some(ref eid) = event_id {
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM affiliate_commissions WHERE metadata->>'event_id' = $1)",
        )
        .bind(eid)
        .fetch_one(&state.pool)
        .await?;
        if already {
            return Ok(Json(json!({"status": "already-credited", "event_id": eid})));
        }
    }

    // Resolve the lead by email — the join key captured when the free account was provisioned.
    let lead: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, created_by FROM leads WHERE email = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await?;
    let Some((lead_id, created_by)) = lead else {
        return Ok(Json(json!({"status": "no-lead", "email": email})));
    };

    // Resolve the affiliate from the lead's originating user account (permanent link).
    let Some(user_id) = created_by else {
        return Ok(Json(json!({"status": "no-affiliate", "email": email})));
    };
    let affiliate: Option<(String, Option<f64>)> = sqlx::query_as(
        "SELECT id, commission_rate FROM affiliates WHERE user_id = $1 AND is_active = true LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((affiliate_id, rate)) = affiliate else {
        return Ok(Json(json!({"status": "no-affiliate", "email": email})));
    };

    // Resolve the affiliate product by source app.
    let product_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM affiliate_products WHERE source_app = $1 AND is_active = true LIMIT 1",
    )
    .bind(source_app)
    .fetch_optional(&state.pool)
    .await?;

    // Commission = upgrade price x the affiliate's plan-derived rate (percentage).
    let commission = plan_price * rate.unwrap_or(0.0) / 100.0;

    let id = Uuid::new_v4();
    let metadata = json!({
        "source_app": source_app,
        "plan_name": plan_name,
        "plan_price": plan_price,
        "event_id": event_id,
    });
    sqlx::query(
        "INSERT INTO affiliate_commissions (id, affiliate_id, lead_id, product_id, amount, status, metadata)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6)",
    )
    .bind(id)
    .bind(&affiliate_id)
    .bind(lead_id)
    .bind(product_id)
    .bind(commission)
    .bind(&metadata)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({
        "status": "credited",
        "id": id.to_string(),
        "affiliate_id": affiliate_id,
        "product_id": product_id.map(|v| v.to_string()),
        "plan_name": plan_name,
        "plan_price": plan_price,
        "commission": commission,
    })))
}
