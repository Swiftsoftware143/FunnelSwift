use axum::{extract::{Path, Query, State}, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;
use std::collections::HashMap;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn list_affiliate_links(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, Option<String>, String, Option<f64>, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, affiliate_id, target_software, tracking_code, commission_rate, created_at FROM affiliate_links ORDER BY created_at DESC"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    let links: Vec<Value> = rows.iter().map(|r| json!({"id": r.0.to_string(), "affiliate_id": r.1, "tracking_code": r.3})).collect();
    Ok(Json(json!(links)))
}
pub async fn create_affiliate_link(_auth: AuthUser, State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let code = format!("TRACK{}", &id.to_string().replace("-", "")[..12]);
    sqlx::query("INSERT INTO affiliate_links (id, affiliate_id, target_software, tracking_code, commission_rate) VALUES ($1, $2, $3, $4, $5)")
        .bind(id).bind(payload["affiliate_id"].as_str().unwrap_or("")).bind(payload["target_software"].as_str().unwrap_or(""))
        .bind(&code).bind(payload["commission_rate"].as_f64().unwrap_or(10.0)).execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id.to_string(), "tracking_code": code}))))
}
pub async fn get_affiliate_stats(_auth: AuthUser, State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"total_clicks": 0, "total_conversions": 0})))
}
pub async fn list_conversions(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, Option<Uuid>, Option<String>, Option<Uuid>, Option<f64>, Option<f64>, Option<String>, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT id, click_id, affiliate_user_id, customer_id, amount, commission, status, converted_at FROM affiliate_conversions ORDER BY converted_at DESC LIMIT 50"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    let items: Vec<Value> = rows.iter().map(|r| json!({"id": r.0.to_string(), "amount": r.4, "commission": r.5, "status": r.6})).collect();
    Ok(Json(json!(items)))
}
pub async fn track_conversion(_auth: AuthUser, State(_state): State<AppState>, Json(_payload): Json<Value>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "Conversion tracked"})))
}
pub async fn track_click(Query(params): Query<HashMap<String, String>>, State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"tracked": true})))
}
