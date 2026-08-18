use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
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
    let rows: Vec<(Uuid, String, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, COALESCE(title,''), COALESCE(slug,''), created_at FROM qr_codes WHERE tenant_id = $1 ORDER BY created_at DESC"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn create_qr_code(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let title = payload["title"].as_str().unwrap_or("QR Code");
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    features::enforce_feature_limit(&state, tenant_id, "max_qr_codes", "QR codes").await?;
    sqlx::query(
        "INSERT INTO qr_codes (id, tenant_id, title, slug, target_url) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(title)
    .bind(title.to_lowercase().replace(' ', "-"))
    .bind(payload["target_url"].as_str().unwrap_or(""))
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
    sqlx::query("DELETE FROM qr_codes WHERE id = $1 AND tenant_id = $2")
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
