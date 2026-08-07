/// ZaarHub City Pages & Directory Listing Handlers
/// Phase 4 — Public API endpoints (no auth required)
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ── Query parameters ───────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListingQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
    pub category: Option<String>,
    pub sort: Option<String>, // rating, name, featured
    pub search: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub city: Option<String>,
    pub page: Option<i32>,
    pub per_page: Option<i32>,
}

#[derive(Deserialize)]
pub struct FeaturedQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
}

// ── Helper ─────────────────────────────────────────────────────

fn default_page(page: Option<i32>) -> i32 { page.unwrap_or(1).max(1) }
fn default_per_page(per_page: Option<i32>) -> i32 { per_page.unwrap_or(20).clamp(1, 100) }
fn offset(page: i32, per_page: i32) -> i32 { (page - 1) * per_page }

/// Utility: build listing JSON from a sqlx::Row
fn listing_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "city_page_id": r.try_get::<Uuid, _>("city_page_id").map(|v| v.to_string()).unwrap_or_default(),
        "business_name": r.try_get::<String, _>("business_name").unwrap_or_default(),
        "category": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "subcategory": r.try_get::<Option<String>, _>("subcategory").unwrap_or_default(),
        "description": r.try_get::<Option<String>, _>("description").unwrap_or_default(),
        "address": r.try_get::<Option<String>, _>("address").unwrap_or_default(),
        "phone": r.try_get::<Option<String>, _>("phone").unwrap_or_default(),
        "website": r.try_get::<Option<String>, _>("website").unwrap_or_default(),
        "logo_url": r.try_get::<Option<String>, _>("logo_url").unwrap_or_default(),
        "cover_image_url": r.try_get::<Option<String>, _>("cover_image_url").unwrap_or_default(),
        "rating": r.try_get::<Option<f64>, _>("rating").unwrap_or_default(),
        "review_count": r.try_get::<i32, _>("review_count").unwrap_or(0),
        "is_featured": r.try_get::<bool, _>("is_featured").unwrap_or(false),
        "is_claimed": r.try_get::<bool, _>("is_claimed").unwrap_or(false),
        "deal_text": r.try_get::<Option<String>, _>("deal_text").unwrap_or_default(),
        "deal_url": r.try_get::<Option<String>, _>("deal_url").unwrap_or_default(),
        "coordinates_lat": r.try_get::<Option<f64>, _>("coordinates_lat").unwrap_or_default(),
        "coordinates_lng": r.try_get::<Option<f64>, _>("coordinates_lng").unwrap_or_default(),
        "display_order": r.try_get::<i32, _>("display_order").unwrap_or(0),
        "created_at": r.try_get::<chrono::NaiveDateTime, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::NaiveDateTime, _>("updated_at").unwrap_or_default(),
    })
}

/// List all active city pages
pub async fn list_cities(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT cp.*, (SELECT COUNT(*) FROM business_listings bl WHERE bl.city_page_id = cp.id) AS listing_count \
         FROM city_pages cp \
         WHERE cp.is_active = true \
         ORDER BY cp.display_order ASC, cp.city_name ASC"
    )
    .fetch_all(&state.pool)
    .await?;

    let cities: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "tenant_id": r.try_get::<Uuid, _>("tenant_id").map(|v| v.to_string()).unwrap_or_default(),
        "city_slug": r.try_get::<String, _>("city_slug").unwrap_or_default(),
        "city_name": r.try_get::<String, _>("city_name").unwrap_or_default(),
        "state": r.try_get::<Option<String>, _>("state").unwrap_or_default(),
        "description": r.try_get::<Option<String>, _>("description").unwrap_or_default(),
        "hero_image_url": r.try_get::<Option<String>, _>("hero_image_url").unwrap_or_default(),
        "meta_title": r.try_get::<Option<String>, _>("meta_title").unwrap_or_default(),
        "meta_description": r.try_get::<Option<String>, _>("meta_description").unwrap_or_default(),
        "is_active": r.try_get::<bool, _>("is_active").unwrap_or(false),
        "display_order": r.try_get::<i32, _>("display_order").unwrap_or(0),
        "listing_count": r.try_get::<i64, _>("listing_count").unwrap_or(0),
        "created_at": r.try_get::<chrono::NaiveDateTime, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::NaiveDateTime, _>("updated_at").unwrap_or_default(),
    })).collect();

    Ok(Json(json!({"cities": cities, "total": cities.len()})))
}

/// Get a single city page by slug (with listing count)
pub async fn get_city(State(state): State<AppState>, Path(slug): Path<String>) -> AppResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT cp.*, (SELECT COUNT(*) FROM business_listings bl WHERE bl.city_page_id = cp.id) AS listing_count \
         FROM city_pages cp \
         WHERE cp.city_slug = $1 AND cp.is_active = true"
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("City page not found: {}", slug)))?;

    let city = json!({
        "id": row.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "tenant_id": row.try_get::<Uuid, _>("tenant_id").map(|v| v.to_string()).unwrap_or_default(),
        "city_slug": row.try_get::<String, _>("city_slug").unwrap_or_default(),
        "city_name": row.try_get::<String, _>("city_name").unwrap_or_default(),
        "state": row.try_get::<Option<String>, _>("state").unwrap_or_default(),
        "description": row.try_get::<Option<String>, _>("description").unwrap_or_default(),
        "hero_image_url": row.try_get::<Option<String>, _>("hero_image_url").unwrap_or_default(),
        "meta_title": row.try_get::<Option<String>, _>("meta_title").unwrap_or_default(),
        "meta_description": row.try_get::<Option<String>, _>("meta_description").unwrap_or_default(),
        "is_active": row.try_get::<bool, _>("is_active").unwrap_or(false),
        "display_order": row.try_get::<i32, _>("display_order").unwrap_or(0),
        "listing_count": row.try_get::<i64, _>("listing_count").unwrap_or(0),
        "created_at": row.try_get::<chrono::NaiveDateTime, _>("created_at").unwrap_or_default(),
        "updated_at": row.try_get::<chrono::NaiveDateTime, _>("updated_at").unwrap_or_default(),
    });

    Ok(Json(json!({"city": city})))
}

/// List businesses for a city with pagination, filtering, and sorting
pub async fn list_city_listings(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<ListingQuery>,
) -> AppResult<Json<Value>> {
    let city_row = sqlx::query("SELECT id FROM city_pages WHERE city_slug = $1 AND is_active = true")
        .bind(&slug)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("City page not found: {}", slug)))?;

    let city_page_id: Uuid = city_row.get("id");
    let page = default_page(params.page);
    let per_page = default_per_page(params.per_page);

    let has_cat = params.category.is_some();
    let has_search = params.search.is_some();

    let (rows, total): (Vec<sqlx::postgres::PgRow>, i64) = match (has_cat, has_search) {
        (false, false) => {
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1")
                .bind(city_page_id)
                .fetch_one(&state.pool).await?;
            let order = match params.sort.as_deref() {
                Some("rating") => "bl.rating DESC NULLS LAST, bl.review_count DESC",
                Some("name") => "bl.business_name ASC",
                Some("featured") => "bl.is_featured DESC, bl.rating DESC NULLS LAST",
                _ => "bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC",
            };
            let rows = sqlx::query(&format!(
                "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 ORDER BY {} LIMIT $2 OFFSET $3", order
            ))
            .bind(city_page_id).bind(per_page as i64).bind(offset(page, per_page) as i64)
            .fetch_all(&state.pool).await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
        (true, false) => {
            let cat = params.category.as_ref().unwrap();
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1 AND category = $2")
                .bind(city_page_id).bind(cat)
                .fetch_one(&state.pool).await?;
            let order = match params.sort.as_deref() {
                Some("rating") => "bl.rating DESC NULLS LAST, bl.review_count DESC",
                Some("name") => "bl.business_name ASC",
                Some("featured") => "bl.is_featured DESC, bl.rating DESC NULLS LAST",
                _ => "bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC",
            };
            let rows = sqlx::query(&format!(
                "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 ORDER BY {} LIMIT $3 OFFSET $4", order
            ))
            .bind(city_page_id).bind(cat).bind(per_page as i64).bind(offset(page, per_page) as i64)
            .fetch_all(&state.pool).await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
        (false, true) => {
            let search = params.search.as_ref().unwrap();
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1 AND (business_name ILIKE '%' || $2 || '%' OR description ILIKE '%' || $2 || '%')")
                .bind(city_page_id).bind(search)
                .fetch_one(&state.pool).await?;
            let order = match params.sort.as_deref() {
                Some("rating") => "bl.rating DESC NULLS LAST, bl.review_count DESC",
                Some("name") => "bl.business_name ASC",
                Some("featured") => "bl.is_featured DESC, bl.rating DESC NULLS LAST",
                _ => "bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC",
            };
            let rows = sqlx::query(&format!(
                "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%') ORDER BY {} LIMIT $3 OFFSET $4", order
            ))
            .bind(city_page_id).bind(search).bind(per_page as i64).bind(offset(page, per_page) as i64)
            .fetch_all(&state.pool).await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
        (true, true) => {
            let cat = params.category.as_ref().unwrap();
            let search = params.search.as_ref().unwrap();
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1 AND category = $2 AND (business_name ILIKE '%' || $3 || '%' OR description ILIKE '%' || $3 || '%')")
                .bind(city_page_id).bind(cat).bind(search)
                .fetch_one(&state.pool).await?;
            let order = match params.sort.as_deref() {
                Some("rating") => "bl.rating DESC NULLS LAST, bl.review_count DESC",
                Some("name") => "bl.business_name ASC",
                Some("featured") => "bl.is_featured DESC, bl.rating DESC NULLS LAST",
                _ => "bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC",
            };
            let rows = sqlx::query(&format!(
                "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 AND (bl.business_name ILIKE '%' || $3 || '%' OR bl.description ILIKE '%' || $3 || '%') ORDER BY {} LIMIT $4 OFFSET $5", order
            ))
            .bind(city_page_id).bind(cat).bind(search).bind(per_page as i64).bind(offset(page, per_page) as i64)
            .fetch_all(&state.pool).await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
    };

    let listings: Vec<Value> = rows.iter().map(listing_json).collect();
    let total_pages = if total == 0 { 0 } else { ((total as f64) / (per_page as f64)).ceil() as i32 };

    Ok(Json(json!({
        "listings": listings,
        "pagination": {
            "page": page,
            "per_page": per_page,
            "total": total,
            "total_pages": total_pages
        }
    })))
}

/// Get single listing detail by id
pub async fn get_listing(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let listing_uuid = Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest("Invalid listing ID".into()))?;

    let row = sqlx::query("SELECT bl.* FROM business_listings bl WHERE bl.id = $1")
        .bind(listing_uuid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Listing not found: {}", id)))?;

    Ok(Json(json!({"listing": listing_json(&row)})))
}

/// List all unique categories across all city listings
pub async fn list_categories(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT DISTINCT category, COUNT(*) AS cnt \
         FROM business_listings \
         WHERE category IS NOT NULL \
         GROUP BY category \
         ORDER BY cnt DESC"
    )
    .fetch_all(&state.pool)
    .await?;

    let categories: Vec<Value> = rows.iter().map(|r| json!({
        "name": r.try_get::<String, _>("category").unwrap_or_default(),
        "count": r.try_get::<i64, _>("cnt").unwrap_or(0),
    })).collect();

    Ok(Json(json!({"categories": categories, "total": categories.len()})))
}

/// Search listings across all cities
pub async fn search_listings(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> AppResult<Json<Value>> {
    let page = default_page(params.page);
    let per_page = default_per_page(params.per_page);
    let search_term = params.q.unwrap_or_default();

    if search_term.is_empty() {
        return Ok(Json(json!({"listings": [], "pagination": {"page": 1, "per_page": per_page, "total": 0, "total_pages": 0}})));
    }

    let has_city = params.city.is_some();

    let (rows, total): (Vec<sqlx::postgres::PgRow>, i64) = if has_city {
        let city = params.city.as_ref().unwrap();
        let count = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM business_listings bl \
             JOIN city_pages cp ON bl.city_page_id = cp.id \
             WHERE cp.city_slug = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%' OR bl.category ILIKE '%' || $2 || '%')"
        ).bind(city).bind(&search_term)
         .fetch_one(&state.pool).await?;
        let rows = sqlx::query(
            "SELECT bl.* FROM business_listings bl \
             JOIN city_pages cp ON bl.city_page_id = cp.id \
             WHERE cp.city_slug = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%' OR bl.category ILIKE '%' || $2 || '%') \
             ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST LIMIT $3 OFFSET $4"
        ).bind(city).bind(&search_term).bind(per_page as i64).bind(offset(page, per_page) as i64)
         .fetch_all(&state.pool).await?;
        let total: i64 = count.get("cnt");
        (rows, total)
    } else {
        let count = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM business_listings bl \
             WHERE bl.business_name ILIKE '%' || $1 || '%' OR bl.description ILIKE '%' || $1 || '%' OR bl.category ILIKE '%' || $1 || '%'"
        ).bind(&search_term)
         .fetch_one(&state.pool).await?;
        let rows = sqlx::query(
            "SELECT bl.* FROM business_listings bl \
             WHERE bl.business_name ILIKE '%' || $1 || '%' OR bl.description ILIKE '%' || $1 || '%' OR bl.category ILIKE '%' || $1 || '%' \
             ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST LIMIT $2 OFFSET $3"
        ).bind(&search_term).bind(per_page as i64).bind(offset(page, per_page) as i64)
         .fetch_all(&state.pool).await?;
        let total: i64 = count.get("cnt");
        (rows, total)
    };

    let listings: Vec<Value> = rows.iter().map(listing_json).collect();
    let total_pages = if total == 0 { 0 } else { ((total as f64) / (per_page as f64)).ceil() as i32 };

    Ok(Json(json!({
        "listings": listings,
        "pagination": {
            "page": page,
            "per_page": per_page,
            "total": total,
            "total_pages": total_pages
        }
    })))
}

/// Get featured listings across all cities
pub async fn featured_listings(
    State(state): State<AppState>,
    Query(q): Query<FeaturedQuery>,
) -> AppResult<Json<Value>> {
    let page = default_page(q.page);
    let per_page = default_per_page(q.per_page);

    let count_row = sqlx::query(
        "SELECT COUNT(*) AS cnt FROM business_listings WHERE is_featured = true"
    )
    .fetch_one(&state.pool)
    .await?;
    let total: i64 = count_row.get("cnt");

    let rows = sqlx::query(
        "SELECT bl.* FROM business_listings bl \
         WHERE bl.is_featured = true \
         ORDER BY bl.rating DESC NULLS LAST, bl.business_name ASC \
         LIMIT $1 OFFSET $2"
    )
    .bind(per_page as i64)
    .bind(offset(page, per_page) as i64)
    .fetch_all(&state.pool)
    .await?;

    let listings: Vec<Value> = rows.iter().map(listing_json).collect();
    let total_pages = if total == 0 { 0 } else { ((total as f64) / (per_page as f64)).ceil() as i32 };

    Ok(Json(json!({
        "listings": listings,
        "pagination": {
            "page": page,
            "per_page": per_page,
            "total": total,
            "total_pages": total_pages
        }
    })))
}
