use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Affiliate {
    pub id: String,
    pub tenant_id: Uuid,
    pub name: String,
    pub email: String,
    pub industry: Option<String>,
    pub commission_rate: Option<f64>,
    pub tax_docs: Option<serde_json::Value>,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAffiliateRequest {
    pub name: String,
    pub email: String,
    pub industry: Option<String>,
    pub commission_rate: Option<f64>,
    pub tax_docs: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateAffiliateRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub industry: Option<String>,
    pub commission_rate: Option<f64>,
    pub tax_docs: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateCommission {
    pub id: Uuid,
    pub affiliate_id: String,
    pub lead_id: Uuid,
    pub amount: f64,
    pub status: String,
    pub paid_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}
