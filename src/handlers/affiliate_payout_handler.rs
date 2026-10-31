use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct CreateTierRequest {
    pub name: String,
    pub commission_rate: Option<f64>,
    pub min_sales: Option<i32>,
    pub min_revenue: Option<f64>,
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateTierRequest {
    pub name: Option<String>,
    pub commission_rate: Option<f64>,
    pub min_sales: Option<i32>,
    pub min_revenue: Option<f64>,
    pub description: Option<String>,
}

pub async fn list_payouts(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    // A NON-admin caller used to get a flat 403 here, while the console renders this route as the
    // tenant's own "My Payouts" screen — so the tenant's payout page always said "No payouts yet"
    // behind a failing request. The screen has always promised a tenant their OWN payouts, so a
    // non-admin now reads ONLY rows keyed to their user id; a platform admin still reads every
    // payout. The write verbs below stay admin-only.
    let own_only = !auth.is_admin;
    let caller = auth.user_id.clone();
    // `affiliate_payouts` (migration 021) is id / affiliate_user_id / amount / method / status /
    // paid_at. The old SELECT asked for p.affiliate_id, p.period and p.created_at -- none of which
    // exist -- so the query errored on EVERY call and unwrap_or_default() reported the failure as
    // `200 []` (kanban t_4385aa2c). amount is NUMERIC while f64 decodes FLOAT8 only, hence the
    // cast; the join keys on affiliates.user_id (affiliates.id is varchar, this column is uuid).
    let sql = if own_only {
        "SELECT p.id, p.affiliate_user_id, p.amount::float8 AS amount, p.status, p.method, p.paid_at, a.name AS affiliate_name \
         FROM affiliate_payouts p LEFT JOIN affiliates a ON a.user_id = p.affiliate_user_id \
         WHERE p.affiliate_user_id = $1::uuid \
         ORDER BY p.paid_at DESC NULLS LAST, p.id"
    } else {
        "SELECT p.id, p.affiliate_user_id, p.amount::float8 AS amount, p.status, p.method, p.paid_at, a.name AS affiliate_name \
         FROM affiliate_payouts p LEFT JOIN affiliates a ON a.user_id = p.affiliate_user_id \
         ORDER BY p.paid_at DESC NULLS LAST, p.id"
    };
    let mut query = sqlx::query_as::<
        _,
        (
            Uuid,
            Option<Uuid>,
            Option<f64>,
            Option<String>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
        ),
    >(sql);
    if own_only {
        query = query.bind(caller);
    }
    let rows = query.fetch_all(&state.pool).await.map_err(|e| {
        tracing::error!("list_payouts query failed: {:?}", e);
        e
    })?;
    let payouts: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.0.to_string(),
                "affiliate_user_id": r.1.map(|u| u.to_string()),
                // legacy key kept for the shipped SPA (`p.affiliate_name || p.affiliate_id`)
                "affiliate_id": r.1.map(|u| u.to_string()),
                "amount": r.2,
                "status": r.3,
                "method": r.4,
                "paid_at": r.5,
                // no period column exists in affiliate_payouts; keep the key the SPA reads
                "period": serde_json::Value::Null,
                "affiliate_name": r.6,
            })
        })
        .collect();
    Ok(Json(json!(payouts)))
}

pub async fn create_payout(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let id = Uuid::new_v4();
    // same column drift as list_payouts: this INSERT named affiliate_id/period, which do not
    // exist on affiliate_payouts, so POST /affiliate-payouts always 500'd (kanban t_4385aa2c)
    let affiliate_user_id = req["affiliate_user_id"]
        .as_str()
        .or_else(|| req["affiliate_id"].as_str())
        .ok_or_else(|| AppError::BadRequest("affiliate_user_id is required".into()))?;
    let affiliate_user_id: Uuid = affiliate_user_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid affiliate_user_id".into()))?;
    // An amount is REQUIRED and must be positive. This used to default to 0.0, so a caller who
    // omitted it recorded a real $0.00 payout row the affiliate then saw in "My Payouts"
    // (measured 2026-10-31: POST with no `amount` answered 201 and stored 0.00).
    let amount = req["amount"]
        .as_f64()
        .filter(|a| a.is_finite() && *a > 0.0)
        .ok_or_else(|| AppError::BadRequest("Enter an amount greater than zero.".into()))?;
    let status = req["status"].as_str().unwrap_or("pending");
    if !matches!(status, "pending" | "paid") {
        return Err(AppError::BadRequest(
            "status must be pending or paid".into(),
        ));
    }
    let method = req["method"].as_str();

    // The id must belong to an affiliate who actually has a linked account. Without this the
    // INSERT hit the `users(id)` foreign key and answered a bare 500 "Database error" for a
    // typo'd or stale id (measured 2026-10-31) — an operator deserves the reason, not a server
    // error. The UI's own picker only offers linked affiliates; the API is now just as strict.
    let linked: Option<(i32,)> = sqlx::query_as(
        "SELECT 1::int FROM affiliates a JOIN users u ON u.id = a.user_id \
         WHERE a.user_id = $1 LIMIT 1",
    )
    .bind(affiliate_user_id)
    .fetch_optional(&state.pool)
    .await?;
    if linked.is_none() {
        return Err(AppError::UnprocessableEntity(
            "That affiliate has no linked user account, so a payout cannot be recorded against them."
                .into(),
        ));
    }

    sqlx::query(
        "INSERT INTO affiliate_payouts (id, affiliate_user_id, amount, method, status) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(affiliate_user_id)
    .bind(amount)
    .bind(method)
    .bind(status)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Payout created"})),
    ))
}

pub async fn mark_payout_paid(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    // A 200 here has to mean a row actually moved. This used to answer
    // {"message":"Payout marked as paid"} for ANY id, including one that does not exist (measured
    // 2026-10-31: 200 on a made-up uuid, 0 rows changed) — so the console could report a payout
    // settled while nothing had happened.
    let res =
        sqlx::query("UPDATE affiliate_payouts SET status = 'paid', paid_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("That payout does not exist.".into()));
    }
    Ok(Json(json!({"message": "Payout marked as paid"})))
}
