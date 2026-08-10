use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct TagGroup {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub is_collapsible: bool,
    pub sort_order: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTagGroupRequest {
    pub name: String,
    pub is_collapsible: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateTagGroupRequest {
    pub name: Option<String>,
    pub is_collapsible: Option<bool>,
    pub sort_order: Option<i32>,
}
