use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateUser {
    pub id: String,
    pub tenant_id: Uuid,
    pub affiliate_id: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub payout_method: Option<String>,
    pub payout_details: Option<String>,
    pub min_payout: Option<f64>,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct AffiliateSignupRequest {
    pub email: String,
    pub password: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub affiliate_code: Option<String>,
    pub selected_apps: Vec<String>,
    pub commission_rate: Option<f64>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AffiliateSelection {
    pub id: Uuid,
    pub affiliate_user_id: String,
    pub app_slug: String,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliatePayout {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub affiliate_id: String,
    pub amount: f64,
    pub method: Option<String>,
    pub status: String,
    pub paid_at: Option<NaiveDateTime>,
    pub notes: Option<String>,
    pub created_at: NaiveDateTime,
}
