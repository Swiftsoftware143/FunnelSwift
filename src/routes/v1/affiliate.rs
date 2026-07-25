use axum::{extract::{Path, Query, State}, Json};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::Database,
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct ListProductsQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
    pub category: Option<String>,
}

pub async fn list_products(
    State(db): State<Database>,
    user: AuthUser,
    Query(query): Query<ListProductsQuery>,
) -> Result<Json<serde_json::Value>> {
    // TODO: Implement affiliate product listing
    Err(AppError::NotImplemented("Affiliate marketplace".to_string()))
}

pub async fn get_product(
    State(db): State<Database>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>> {
    // TODO: Implement affiliate product details
    Err(AppError::NotImplemented("Affiliate product details".to_string()))
}