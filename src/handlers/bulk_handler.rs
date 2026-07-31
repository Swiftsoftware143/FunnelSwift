// Bulk operations handler
use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct BulkDeleteRequest {
    pub ids: Vec<String>,
}

pub async fn bulk_delete_leads(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BulkDeleteRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let mut count: i64 = 0;
    for id_str in &req.ids {
        if let Ok(id) = Uuid::parse_str(id_str) {
            let result = sqlx::query("DELETE FROM leads WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id)
                .execute(&state.pool)
                .await?;
            count += result.rows_affected() as i64;
        }
    }

    Ok(Json(json!({"deleted": count, "message": format!("{} leads deleted", count)})))
}

pub async fn bulk_delete_affiliates(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BulkDeleteRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let mut count: i64 = 0;
    for id_str in &req.ids {
        let result = sqlx::query("DELETE FROM affiliates WHERE id = $1 AND tenant_id = $2")
            .bind(id_str)
            .bind(tenant_id)
            .execute(&state.pool)
            .await?;
        count += result.rows_affected() as i64;
    }

    Ok(Json(json!({"deleted": count, "message": format!("{} affiliates deleted", count)})))
}

pub async fn bulk_delete_users(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BulkDeleteRequest>,
) -> AppResult<Json<Value>> {
    let mut count: i64 = 0;
    for id_str in &req.ids {
        if let Ok(id) = Uuid::parse_str(id_str) {
            let result = sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(id)
                .execute(&state.pool)
                .await?;
            count += result.rows_affected() as i64;
        }
    }

    Ok(Json(json!({"deleted": count, "message": format!("{} users deleted", count)})))
}

pub async fn bulk_delete_products(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BulkDeleteRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let mut count: i64 = 0;
    for id_str in &req.ids {
        if let Ok(id) = Uuid::parse_str(id_str) {
            let result = sqlx::query("DELETE FROM affiliate_products WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id)
                .execute(&state.pool)
                .await?;
            count += result.rows_affected() as i64;
        }
    }

    Ok(Json(json!({"deleted": count, "message": format!("{} products deleted", count)})))
}
