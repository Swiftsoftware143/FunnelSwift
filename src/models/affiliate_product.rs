use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateProduct {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub is_active: bool,
    pub is_third_party: Option<bool>,
    pub url: Option<String>,
    pub category_id: Option<Uuid>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAffiliateProductRequest {
    pub name: String,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub category_id: Option<Uuid>,
    pub url: Option<String>,
    pub is_third_party: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateAffiliateProductRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub default_commission_rate: Option<f64>,
    pub is_active: Option<bool>,
    pub category_id: Option<Uuid>,
    pub url: Option<String>,
    pub is_third_party: Option<bool>,
}
