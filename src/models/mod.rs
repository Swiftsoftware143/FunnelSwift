pub mod activity;
pub mod affiliate;
pub mod api_key;
pub mod dashboard;
pub mod lead;
pub mod plan;
pub mod plan_tag_mapping;
pub mod routing;
pub mod setting;
pub mod settings;
pub mod tag;
pub mod tag_group;
pub mod user;
pub mod webhook;
pub mod workflow;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Timestamps {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
    pub success: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: i64,
    pub page: i32,
    pub per_page: i32,
    pub total_pages: i32,
}