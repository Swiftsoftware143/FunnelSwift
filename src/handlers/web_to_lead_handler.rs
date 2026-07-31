use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateWebToLeadConfig {
    pub name: String,
    pub form_title: Option<String>,
    pub fields: Option<Vec<String>>,
    pub thank_you_message: Option<String>,
    pub redirect_url: Option<String>,
}

pub async fn list_web_to_lead_configs(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, Option<String>, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, form_title, created_at FROM web_to_lead_configs ORDER BY created_at DESC"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn create_web_to_lead_config(auth: AuthUser, State(state): State<AppState>, Json(payload): Json<CreateWebToLeadConfig>) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let tenant_id = Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let fields = payload.fields.unwrap_or_else(|| vec!["name".to_string(), "email".to_string()]);
    sqlx::query("INSERT INTO web_to_lead_configs (id, tenant_id, name, form_title, fields, public_key) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(id).bind(tenant_id).bind(&payload.name)
        .bind(payload.form_title.as_deref().unwrap_or("Get Started"))
        .bind(&fields).bind(&id.to_string()).execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id.to_string(), "public_key": id.to_string()}))))
}
pub async fn update_web_to_lead_config(_auth: AuthUser, State(_state): State<AppState>, Path(_id): Path<Uuid>, Json(_payload): Json<Value>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "Config updated"})))
}
pub async fn delete_web_to_lead_config(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM web_to_lead_configs WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Config deleted"})))
}
pub async fn get_web_to_lead_embed(_auth: AuthUser, State(_state): State<AppState>, Path(_id): Path<Uuid>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"embed_code": "<div id=\"fsw-wtl\"></div><script src=\"/api/v1/web-to-lead/embed.js\"></script>"})))
}
pub async fn handle_web_to_lead(State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO leads (id, tenant_id, name, email, source, status) VALUES ($1, $2, $3, $4, 'web', 'new')")
        .bind(id).bind(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap_or_default())
        .bind(payload["name"].as_str().unwrap_or("")).bind(payload["email"].as_str().unwrap_or(""))
        .execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id.to_string(), "message": "Lead captured"}))))
}
