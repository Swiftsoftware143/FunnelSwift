use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn list_funnels(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, COALESCE(slug,''), created_at FROM funnels ORDER BY created_at DESC"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn create_funnel(auth: AuthUser, State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let name = payload["name"].as_str().unwrap_or("New Funnel");
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("INSERT INTO funnels (id, tenant_id, name, slug) VALUES ($1, $2, $3, $4)")
        .bind(id).bind(tenant_id).bind(name).bind(name.to_lowercase().replace(' ', "-")).execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id.to_string(), "message": "Funnel created"}))))
}
pub async fn get_funnel(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let row: Option<(Uuid, String, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, COALESCE(slug,''), created_at FROM funnels WHERE id = $1"
    ).bind(id).fetch_optional(&state.pool).await?;
    let r = row.ok_or_else(|| AppError::NotFound("Funnel not found".into()))?;
    Ok(Json(json!({"id": r.0.to_string(), "name": r.1, "slug": r.2})))
}
pub async fn update_funnel(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>, Json(payload): Json<Value>) -> AppResult<Json<Value>> {
    if let Some(name) = payload["name"].as_str() { sqlx::query("UPDATE funnels SET name=$1 WHERE id=$2").bind(name).bind(id).execute(&state.pool).await?; }
    Ok(Json(json!({"message": "Funnel updated"})))
}
pub async fn delete_funnel(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM funnels WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Funnel deleted"})))
}
pub async fn render_funnel(axum::extract::Path(slug): axum::extract::Path<String>, State(_state): State<AppState>) -> axum::response::Html<String> {
    axum::response::Html(format!("<h1>Funnel: {}</h1><p>Redirecting...</p>", slug))
}
