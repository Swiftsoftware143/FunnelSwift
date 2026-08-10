use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardStats {
    pub total_leads: i64,
    pub leads_by_stage: Vec<StageCount>,
    pub leads_by_source: Vec<SourceCount>,
    pub conversion_rate: f64,
    pub leads_today: i64,
    pub leads_this_week: i64,
    pub leads_this_month: i64,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct StageCount {
    pub stage: String,
    pub count: i64,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct SourceCount {
    pub source: String,
    pub count: i64,
}
