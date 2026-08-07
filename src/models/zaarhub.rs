use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;

/// Represents a city/region page in the ZaarHub directory system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CityPage {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub city_slug: String,
    pub city_name: String,
    pub state: Option<String>,
    pub description: Option<String>,
    pub hero_image_url: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub is_active: bool,
    pub display_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Represents a business listing on a ZaarHub city page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessListing {
    pub id: Uuid,
    pub city_page_id: Uuid,
    pub business_name: String,
    pub category: Option<String>,
    pub subcategory: Option<String>,
    pub description: Option<String>,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub logo_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub rating: Option<f64>,
    pub review_count: i32,
    pub is_featured: bool,
    pub is_claimed: bool,
    pub deal_text: Option<String>,
    pub deal_url: Option<String>,
    pub coordinates_lat: Option<f64>,
    pub coordinates_lng: Option<f64>,
    pub display_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
