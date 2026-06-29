//! Stub handler

use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub search: Option<String>,
}

pub async fn list(
    State(_state): State<AppState>,
    Query(_query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!([])))
}

pub async fn create(
    State(_state): State<AppState>,
    Json(_body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({})))
}

pub async fn get(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({})))
}

pub async fn update(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
    Json(_body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({})))
}

pub async fn delete(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({})))
}
