//! QR Code Handler for Kinetic Cards
//!
//! Generates QR codes pointing to card URLs, manages named QR codes per card,
//! enforces plan-based limits (Free: 1, Upgraded: unlimited), and serves
//! downloadable SVG/PNG QR code images.

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    Json,
};
use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ──────────────────────────────────────────────
// STRUCTS
// ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct KineticQrCode {
    pub id: Uuid,
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub card_id: Uuid,
    pub name: String,
    pub is_active: bool,
    pub download_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateQrInput {
    pub card_id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateQrInput {
    pub name: Option<String>,
    pub is_active: Option<bool>,
}

// ──────────────────────────────────────────────
// HELPERS
// ──────────────────────────────────────────────

/// Get the plan limit for QR codes
async fn get_qr_limit(pool: &PgPool, tenant_id: Uuid) -> i64 {
    let features_row: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT pp.features FROM tenant_plan_subscriptions tps
           JOIN plans pp ON pp.id = tps.plan_id
           WHERE tps.tenant_id = $1 AND tps.status = 'active' LIMIT 1"#
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let f = features_row.unwrap_or_default();
    f.get("max_qr_codes").and_then(|v| v.as_i64()).unwrap_or(1)
}

/// Build the public card URL from slug
fn card_url(slug: &str) -> String {
    format!("https://funnelswift.net/k/{}", slug)
}

/// Generate QR code as SVG string
fn generate_qr_svg(url: &str) -> Result<String, AppError> {
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| AppError::Internal(format!("QR generation failed: {}", e)))?;

    let svg = code.render()
        .min_dimensions(4, 4)
        .dark_color(qrcode::render::svg::Color("#0F172A"))
        .light_color(qrcode::render::svg::Color("#FFFFFF"))
        .build();

    Ok(svg)
}

/// Generate QR code as PNG bytes using the image crate
fn generate_qr_png(url: &str) -> Result<Vec<u8>, AppError> {
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| AppError::Internal(format!("QR generation failed: {}", e)))?;

    // Render as SVG first, then build a simple pixel-based PNG
    // Using the renderer directly with custom pixel type
    let img = code.render::<image::Luma<u8>>()
        .min_dimensions(4, 4)
        .dark_color(image::Luma([15u8]))
        .light_color(image::Luma([255u8]))
        .build();

    let (w, h) = img.dimensions();
    let scale = 8u32;
    let border = 20u32;
    let out_w = w * scale + border * 2;
    let out_h = h * scale + border * 2;

    let mut out = image::RgbImage::new(out_w, out_h);
    // Fill white
    for pixel in out.pixels_mut() {
        *pixel = image::Rgb([255, 255, 255]);
    }

    for y in 0..h {
        for x in 0..w {
            let px = img.get_pixel(x, y).0[0];
            if px < 128 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let py = y * scale + border + dy;
                        let px2 = x * scale + border + dx;
                        if py < out_h && px2 < out_w {
                            out.put_pixel(px2, py, image::Rgb([15, 23, 42]));
                        }
                    }
                }
            }
        }
    }

    let mut bytes = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| AppError::Internal(format!("PNG encode failed: {}", e)))?;

    Ok(bytes)
}

/// Enforce QR code plan limit
async fn enforce_qr_limit(pool: &PgPool, tenant_id: Uuid) -> AppResult<()> {
    let limit = get_qr_limit(pool, tenant_id).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_qr_codes WHERE tenant_id = $1 AND is_active = true"
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB count failed: {}", e)))?;

    if count >= limit {
        return Err(AppError::Forbidden(format!(
            "Plan limit: max {} QR codes. Upgrade your plan to create more.", limit
        )));
    }
    Ok(())
}

/// Helper to parse user info from auth
struct AuthInfo {
    user_id: Uuid,
    tenant_id: Uuid,
}

fn parse_auth(auth: &AuthUser) -> Result<AuthInfo, AppError> {
    let user_id = Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::BadRequest("Invalid user ID".into()))?;
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant ID".into()))?;
    Ok(AuthInfo { user_id, tenant_id })
}

// ──────────────────────────────────────────────
// HANDLERS
// ──────────────────────────────────────────────

/// List QR codes for the authenticated user's tenant
pub async fn list_qr_codes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Value>> {
    let info = parse_auth(&auth)?;

    let rows = sqlx::query_as::<_, KineticQrCode>(
        r#"SELECT q.* FROM kinetic_qr_codes q
           JOIN kinetic_cards c ON c.id = q.card_id
           WHERE q.tenant_id = $1
           ORDER BY q.created_at DESC"#
    )
    .bind(info.tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB list failed: {}", e)))?;

    let mut results = Vec::new();
    for row in rows {
        let card_info: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
            "SELECT slug, title FROM kinetic_cards WHERE id = $1"
        )
        .bind(row.card_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

        let (card_slug, card_title) = card_info.unwrap_or_default();

        results.push(json!({
            "id": row.id,
            "card_id": row.card_id,
            "name": row.name,
            "url": card_url(&card_slug),
            "is_active": row.is_active,
            "download_count": row.download_count,
            "created_at": row.created_at,
            "updated_at": row.updated_at,
            "card_slug": card_slug,
            "card_title": card_title,
        }));
    }

    Ok(Json(json!({
        "success": true,
        "data": results,
        "limit": get_qr_limit(&state.pool, info.tenant_id).await
    })))
}

/// Create a new QR code for a kinetic card
pub async fn create_qr_code(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<CreateQrInput>,
) -> AppResult<Json<Value>> {
    let info = parse_auth(&auth)?;
    let card_id = Uuid::parse_str(&input.card_id)
        .map_err(|_| AppError::Validation("Invalid card ID format".into()))?;

    // Verify card exists and belongs to this tenant
    let card_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kinetic_cards WHERE id = $1 AND tenant_id = $2)"
    )
    .bind(card_id)
    .bind(info.tenant_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB check failed: {}", e)))?;

    if !card_exists {
        return Err(AppError::NotFound("Kinetic card not found".into()));
    }

    let name = input.name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::Validation("QR code name must be 1-100 characters".into()));
    }

    enforce_qr_limit(&state.pool, info.tenant_id).await?;

    let id = Uuid::new_v4();
    let now = chrono::Utc::now();

    sqlx::query(
        r#"INSERT INTO kinetic_qr_codes (id, user_id, tenant_id, card_id, name, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#
    )
    .bind(id)
    .bind(info.user_id)
    .bind(info.tenant_id)
    .bind(card_id)
    .bind(&name)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB insert failed: {}", e)))?;

    let slug: String = sqlx::query_scalar("SELECT slug FROM kinetic_cards WHERE id = $1")
        .bind(card_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("DB slug lookup failed: {}", e)))?;

    Ok(Json(json!({
        "success": true,
        "data": {
            "id": id,
            "card_id": card_id,
            "name": name,
            "url": card_url(&slug),
            "is_active": true,
            "download_count": 0,
            "created_at": now,
            "updated_at": now,
        }
    })))
}

/// Update a QR code (name, active status)
pub async fn update_qr_code(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateQrInput>,
) -> AppResult<Json<Value>> {
    let info = parse_auth(&auth)?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kinetic_qr_codes WHERE id = $1 AND tenant_id = $2)"
    )
    .bind(id)
    .bind(info.tenant_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB check failed: {}", e)))?;

    if !exists {
        return Err(AppError::NotFound("QR code not found".into()));
    }

    if let Some(ref name) = input.name {
        let n = name.trim().to_string();
        if n.is_empty() || n.len() > 100 {
            return Err(AppError::Validation("QR code name must be 1-100 characters".into()));
        }
        sqlx::query("UPDATE kinetic_qr_codes SET name = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(&n)
            .bind(id)
            .bind(info.tenant_id)
            .execute(&state.pool)
            .await
            .map_err(|e| AppError::Internal(format!("DB update failed: {}", e)))?;
    }

    if let Some(active) = input.is_active {
        sqlx::query("UPDATE kinetic_qr_codes SET is_active = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(active)
            .bind(id)
            .bind(info.tenant_id)
            .execute(&state.pool)
            .await
            .map_err(|e| AppError::Internal(format!("DB update failed: {}", e)))?;
    }

    Ok(Json(json!({ "success": true })))
}

/// Delete a QR code
pub async fn delete_qr_code(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let info = parse_auth(&auth)?;

    let result = sqlx::query(
        "DELETE FROM kinetic_qr_codes WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id)
    .bind(info.tenant_id)
    .execute(&state.pool)
    .await
    .map_err(|e| AppError::Internal(format!("DB delete failed: {}", e)))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("QR code not found".into()));
    }

    Ok(Json(json!({ "success": true })))
}

/// Serve QR code SVG — increments download count
pub async fn get_qr_svg(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let db_result = sqlx::query_as::<_, KineticQrCode>(
        "SELECT * FROM kinetic_qr_codes WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await;

    let qr = match db_result {
        Ok(Some(q)) => q,
        Ok(None) => return (StatusCode::NOT_FOUND, "QR code not found").into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "DB error").into_response(),
    };

    let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM kinetic_cards WHERE id = $1")
        .bind(qr.card_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

    let url = slug.as_deref().map(card_url).unwrap_or_else(|| "https://funnelswift.net".into());

    let svg_inner = match generate_qr_svg(&url) {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "QR generation failed").into_response(),
    };

    // Increment download count
    let _ = sqlx::query(
        "UPDATE kinetic_qr_codes SET download_count = download_count + 1 WHERE id = $1"
    )
    .bind(id)
    .execute(&state.pool)
    .await;

    let svg_full = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 300" width="300" height="300">
  <rect width="300" height="300" fill="white" rx="12"/>
  <g transform="translate(20, 20)">{}</g>
</svg>"#,
        svg_inner
    );

    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/svg+xml")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(svg_full))
    {
        Ok(resp) => resp,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)).into_response(),
    }
}

/// Download QR code as PNG file — increments download count
pub async fn get_qr_png(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let db_result = sqlx::query_as::<_, KineticQrCode>(
        "SELECT * FROM kinetic_qr_codes WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await;

    let qr = match db_result {
        Ok(Some(q)) => q,
        Ok(None) => return (StatusCode::NOT_FOUND, "QR code not found").into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "DB error").into_response(),
    };

    let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM kinetic_cards WHERE id = $1")
        .bind(qr.card_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

    let url = slug.as_deref().map(card_url).unwrap_or_else(|| "https://funnelswift.net".into());

    let png_bytes = match generate_qr_png(&url) {
        Ok(b) => b,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "QR generation failed").into_response(),
    };

    // Increment download count
    let _ = sqlx::query(
        "UPDATE kinetic_qr_codes SET download_count = download_count + 1 WHERE id = $1"
    )
    .bind(id)
    .execute(&state.pool)
    .await;

    let safe_name = qr.name.replace(' ', "_");
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.png\"", safe_name))
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(png_bytes))
    {
        Ok(resp) => resp,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)).into_response(),
    }
}
