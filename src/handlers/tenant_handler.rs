use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde_json::json;
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn list_tenants(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin { return Err(AppError::Forbidden("Admin only".into())); }
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>, Option<String>, Option<bool>, chrono::NaiveDateTime)>(
        "SELECT t.id, t.name, t.email, t.status, t.plan_id, p.name as plan_name, t.is_visible, t.created_at FROM tenants t LEFT JOIN plans p ON t.plan_id = p.id ORDER BY t.created_at DESC"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    let tenants: Vec<serde_json::Value> = rows.into_iter().map(|r| json!({
        "id": r.0, "name": r.1, "email": r.2, "status": r.3, "plan_id": r.4, "plan_name": r.5, "is_visible": r.6, "created_at": r.7
    })).collect();
    Ok(Json(json!(tenants)))
}

pub async fn get_tenant(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!({"id": id})))
}

pub async fn create_tenant(auth: AuthUser, State(state): State<AppState>, Json(req): Json<serde_json::Value>) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin { return Err(AppError::Forbidden("Admin only".into())); }
    let id = Uuid::new_v4();
    let name = req["name"].as_str().unwrap_or("New Tenant");
    let email = req["email"].as_str().unwrap_or("");
    sqlx::query("INSERT INTO tenants (id, name, email) VALUES ($1, $2, $3)").bind(id).bind(name).bind(email).execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "message": "Tenant created"}))))
}

pub async fn update_tenant(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>, Json(req): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin { return Err(AppError::Forbidden("Admin only".into())); }
    sqlx::query("UPDATE tenants SET name=COALESCE($2, name), email=COALESCE($3, email) WHERE id=$1")
        .bind(id).bind(req["name"].as_str()).bind(req["email"].as_str()).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Tenant updated"})))
}

pub async fn delete_tenant(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin { return Err(AppError::Forbidden("Admin only".into())); }
    sqlx::query("DELETE FROM tenants WHERE id=$1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Tenant deleted"})))
}

pub async fn assign_plan(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>, Json(req): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin { return Err(AppError::Forbidden("Admin only".into())); }
    let plan_id = req["plan_id"].as_str().unwrap_or("");
    sqlx::query("UPDATE tenants SET plan_id=$2 WHERE id=$1").bind(id).bind(plan_id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Plan assigned"})))
}

pub async fn get_tenant_credits(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!({"credits": 0})))
}

pub async fn assign_credits(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>, Json(req): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!({"message": "Credits assigned"})))
}
