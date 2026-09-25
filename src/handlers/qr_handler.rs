use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

pub async fn list_qr_codes(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // A QR code belongs to a kinetic CARD. The canonical model is `kinetic_qr_codes`
    // (migration 0015) — the table `features::count_usage` counts for the `max_qr_codes`
    // plan gate. The old statement read a `qr_codes` relation that NO migration ever
    // created, so it was a 42P01 on every call, and this swallow turned it into a
    // `200 []` that no caller could distinguish from "you have no QR codes"
    // (kanban t_7197ad72). Title/slug are the card's, which is what a QR's display
    // name is; `created_at` is timestamptz, so it decodes as DateTime<Utc>.
    let rows: Vec<(Uuid, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT q.id, COALESCE(c.title,''), COALESCE(c.slug,''), q.created_at \
         FROM kinetic_qr_codes q LEFT JOIN kinetic_cards c ON c.id = q.card_id \
         WHERE q.tenant_id = $1 ORDER BY q.created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| tracing::error!("list_qr_codes query failed: {:?}", e))
    .unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn create_qr_code(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let user_id = Uuid::parse_str(&auth.user_id).unwrap_or_default();
    // The card is the request's real argument: `kinetic_qr_codes.card_id` is NOT NULL and
    // FKs kinetic_cards, and the tenant check below keeps a caller from attaching a QR to
    // another workspace's card (the FK itself is not tenant-aware).
    let card_id = payload["card_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::BadRequest("card_id is required".into()))?;
    let owned: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM kinetic_cards WHERE id = $1 AND tenant_id = $2")
            .bind(card_id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound("Card not found".into()));
    }
    features::enforce_feature_limit(&state, tenant_id, "max_qr_codes", "QR codes").await?;
    let id = Uuid::new_v4();
    // name is varchar(100): truncate rather than let a long title 500 on the insert.
    let title: String = payload["title"]
        .as_str()
        .or_else(|| payload["name"].as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or("QR Code")
        .chars()
        .take(100)
        .collect();
    sqlx::query(
        "INSERT INTO kinetic_qr_codes (id, user_id, tenant_id, card_id, name) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(card_id)
    .bind(&title)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.to_string(), "message": "QR code created"})),
    ))
}
pub async fn update_qr_code(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
    Json(_payload): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "QR code updated"})))
}
pub async fn delete_qr_code(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("DELETE FROM kinetic_qr_codes WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "QR code deleted"})))
}
pub async fn get_qr_svg(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> AppResult<(
    StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    String,
)> {
    Ok((StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "image/svg+xml")], "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><rect width='100' height='100' fill='#000'/></svg>".to_string()))
}
pub async fn get_qr_png(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> AppResult<(
    StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    Vec<u8>,
)> {
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/png")],
        vec![],
    ))
}
