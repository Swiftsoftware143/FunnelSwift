// Affiliate lead handler
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;

pub async fn submit_affiliate_lead(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let tenant_id = Uuid::parse_str(
        payload["tenant_id"]
            .as_str()
            .unwrap_or("00000000-0000-0000-0000-000000000001"),
    )
    .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("INSERT INTO leads (id, tenant_id, name, email, phone, source, status, custom_fields) VALUES ($1, $2, $3, $4, $5, 'affiliate', 'new', $6)")
        .bind(id).bind(tenant_id).bind(payload["name"].as_str().unwrap_or(""))
        .bind(payload["email"].as_str().unwrap_or("")).bind(payload["phone"].as_str().unwrap_or(""))
        .bind(&payload["custom_fields"]).execute(&state.pool).await?;
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
    let rows: Vec<(Uuid, String, Option<String>, Option<String>, Option<String>, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, email, phone, source, status, created_at FROM leads WHERE tenant_id = $1 AND source = 'affiliate' ORDER BY created_at DESC LIMIT 50"
    ).bind(tenant_id).fetch_all(&state.pool).await?;
    let leads: Vec<Value> = rows.iter().map(|r| json!({"id": r.0.to_string(), "name": r.1, "email": r.2, "phone": r.3, "source": r.4, "status": r.5, "created_at": r.6})).collect();
    Ok(Json(json!(leads)))
}

pub async fn get_affiliate_leads_stats(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"total": 0, "converted": 0, "pending": 0})))
}

pub async fn check_affiliate_for_email(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let row: Option<(String,)> = sqlx::query_as("SELECT id FROM affiliates WHERE email = $1")
        .bind(email)
        .fetch_optional(&state.pool)
        .await?;
    Ok(Json(json!({"exists": row.is_some()})))
}

pub async fn log_lead_movement(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let lead_id_str = payload["lead_id"].as_str().unwrap_or("");
    let to_stage = payload["to_stage"].as_str().unwrap_or("");
    if let Ok(lead_id) = Uuid::parse_str(lead_id_str) {
        sqlx::query("UPDATE leads SET stage = $1, updated_at = NOW() WHERE id = $2")
            .bind(to_stage)
            .bind(lead_id)
            .execute(&state.pool)
            .await?;
    }
    Ok(Json(json!({"message": "Movement logged"})))
}
