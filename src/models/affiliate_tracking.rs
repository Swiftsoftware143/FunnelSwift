use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateLink {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub affiliate_id: String,
    pub product_id: Option<Uuid>,
    pub target_app: Option<String>,
    pub target_url: String,
    pub code: String,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAffiliateLinkRequest {
    pub affiliate_id: String,
    pub product_id: Option<Uuid>,
    pub target_app: Option<String>,
    pub target_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateAffiliateLinkRequest {
    pub is_active: Option<bool>,
    pub target_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateClick {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub affiliate_id: String,
    pub product_id: Option<Uuid>,
    pub target_app: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub landing_page: Option<String>,
    pub cookie_id: Option<String>,
    pub clicked_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateConversion {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub affiliate_id: String,
    pub product_id: Option<Uuid>,
    pub lead_id: Option<Uuid>,
    pub source_app: Option<String>,
    pub commission_amount: Option<f64>,
    pub commission_rate: Option<f64>,
    pub status: String,
    pub cookie_id: Option<String>,
    pub converted_at: NaiveDateTime,
    pub paid_at: Option<NaiveDateTime>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AffiliateStats {
    pub total_clicks: i64,
    pub total_conversions: i64,
    pub total_pending: i64,
    pub total_paid: i64,
    pub total_commission: f64,
    pub conversion_rate: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackConversionRequest {
    pub affiliate_id: String,
    pub lead_id: Option<Uuid>,
    pub product_id: Option<Uuid>,
    pub source_app: Option<String>,
    pub commission_amount: Option<f64>,
    pub commission_rate: Option<f64>,
    pub cookie_id: Option<String>,
}
