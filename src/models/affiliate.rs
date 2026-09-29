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
    /// A deliberate per-person override. Distinct from `commission_rate` on purpose: "their standing
    /// rate is 10%" and "they were given 25% as an override" must both stay recoverable. Wins over
    /// every product, group and plan rate.
    pub override_commission_rate: Option<f64>,
    /// Why the override exists, in the admin's own words.
    pub override_note: Option<String>,
    pub tax_docs: Option<serde_json::Value>,
    pub is_active: bool,
    pub is_visible: Option<bool>,
    pub tags: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAffiliateRequest {
    pub name: String,
    pub email: String,
    pub industry: Option<String>,
    pub commission_rate: Option<f64>,
    pub override_commission_rate: Option<f64>,
    pub override_note: Option<String>,
    pub tax_docs: Option<serde_json::Value>,
    pub tags: Option<serde_json::Value>,
    pub is_visible: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateAffiliateRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub industry: Option<String>,
    pub commission_rate: Option<f64>,
    pub override_commission_rate: Option<f64>,
    pub override_note: Option<String>,
    /// Explicitly remove the override. Needed because "send null" and "leave it alone" are the same
    /// JSON (`None`) — without this a raised rate could be set and never taken away.
    pub clear_override: Option<bool>,
    pub tax_docs: Option<serde_json::Value>,
    pub is_active: Option<bool>,
    pub tags: Option<serde_json::Value>,
    pub is_visible: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AffiliateCommission {
    pub id: Uuid,
    pub affiliate_id: String,
    pub lead_id: Option<Uuid>,
    pub amount: f64,
    pub status: String,
    pub paid_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}
