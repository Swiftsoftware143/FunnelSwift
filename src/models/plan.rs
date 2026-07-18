use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Plan {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub price: f64,
    pub purchase_url: Option<String>,
    pub max_leads: Option<i32>,
    pub max_tags: Option<i32>,
    pub has_dual_routing: bool,
    pub has_multi_tenant: bool,
    pub has_white_label: bool,
    pub payment_provider: Option<String>,
    pub features: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePlanRequest {
    pub name: String,
    pub slug: String,
    pub price: f64,
    pub purchase_url: Option<String>,
    pub max_leads: Option<i32>,
    pub max_tags: Option<i32>,
    pub has_dual_routing: Option<bool>,
    pub has_multi_tenant: Option<bool>,
    pub has_white_label: Option<bool>,
    pub payment_provider: Option<String>,
    pub features: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePlanRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub price: Option<f64>,
    pub purchase_url: Option<String>,
    pub max_leads: Option<i32>,
    pub max_tags: Option<i32>,
    pub has_dual_routing: Option<bool>,
    pub has_multi_tenant: Option<bool>,
    pub has_white_label: Option<bool>,
    pub payment_provider: Option<String>,
    pub features: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct TenantPlanSubscription {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub plan_id: Uuid,
    pub start_date: NaiveDateTime,
    pub end_date: Option<NaiveDateTime>,
    pub status: String,
    pub created_at: NaiveDateTime,
}
