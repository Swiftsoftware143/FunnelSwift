use axum::{extract::{Query, State}, Json};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::affiliate_tracking::*;
use crate::AppState;
use serde_json::{json, Value};

pub async fn list_affiliate_links(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let links = sqlx::query_as::<_, AffiliateLink>(
        "SELECT id, tenant_id, affiliate_id, product_id, target_app, target_url, code, is_active, created_at 
         FROM affiliate_links WHERE tenant_id = $1 ORDER BY created_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": links})))
}

pub async fn create_affiliate_link(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAffiliateLinkRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
        // Generate unique code
    let code = format!("ref-{}", &Uuid::new_v4().to_string()[..8]);
    
    // Build target URL
    let target_url = req.target_url.unwrap_or_else(|| "https://app.funnelswift.net".to_string());
    
    let link = sqlx::query_as::<_, AffiliateLink>(
        "INSERT INTO affiliate_links (tenant_id, affiliate_id, product_id, target_app, target_url, code)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, tenant_id, affiliate_id, product_id, target_app, target_url, code, is_active, created_at"
    )
    .bind(tenant_id)
    .bind(&req.affiliate_id)
    .bind(req.product_id)
    .bind(&req.target_app)
    .bind(&target_url)
    .bind(&code)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"data": link})))
}

pub async fn get_affiliate_stats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
    let clicks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_clicks WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    let conversions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_conversions WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_conversions WHERE tenant_id = $1 AND status = 'pending'"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    let paid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_conversions WHERE tenant_id = $1 AND status = 'paid'"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    let total_commission: Option<f64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(commission_amount), 0) FROM affiliate_conversions WHERE tenant_id = $1 AND status = 'paid'"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    
    let conv_rate = if clicks > 0 { (conversions as f64 / clicks as f64) * 100.0 } else { 0.0 };
    
    Ok(Json(json!({
        "total_clicks": clicks,
        "total_conversions": conversions,
        "total_pending": pending,
        "total_paid": paid,
        "total_commission": total_commission.unwrap_or(0.0),
        "conversion_rate": (conv_rate * 100.0).round() / 100.0
    })))
}

pub async fn track_click(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let code = params.get("ref").ok_or_else(|| AppError::BadRequest("Missing ref parameter".into()))?;
    
    let link = sqlx::query_as::<_, AffiliateLink>(
        "SELECT id, tenant_id, affiliate_id, product_id, target_app, target_url, code, is_active, created_at 
         FROM affiliate_links WHERE code = $1 AND is_active = true"
    )
    .bind(code)
    .fetch_optional(&state.pool)
    .await?;
    
    match link {
        Some(l) => {
            // Record click
            sqlx::query(
                "INSERT INTO affiliate_clicks (tenant_id, affiliate_id, product_id, target_app, ip_address, user_agent, referer, landing_page, cookie_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
            )
            .bind(l.tenant_id)
            .bind(&l.affiliate_id)
            .bind(l.product_id)
            .bind(&l.target_app)
            .bind(params.get("ip"))
            .bind(params.get("ua"))
            .bind(params.get("ref"))
            .bind(params.get("page"))
            .bind(params.get("cid"))
            .execute(&state.pool)
            .await?;
            
            Ok(Json(json!({"redirect": l.target_url, "affiliate_id": l.affiliate_id})))
        }
        None => Err(AppError::NotFound("Invalid referral code".into()))
    }
}

pub async fn track_conversion(
    State(state): State<AppState>,
    Json(req): Json<TrackConversionRequest>,
) -> AppResult<Json<Value>> {
    // This route is called by other apps via webhook or internally
    // Find the tenant by affiliate_id
    let affiliate: Option<(Uuid,)> = sqlx::query_as(
        "SELECT tenant_id FROM affiliates WHERE id = $1"
    )
    .bind(&req.affiliate_id)
    .fetch_optional(&state.pool)
    .await?;
    
    let (tenant_id,) = affiliate.ok_or_else(|| AppError::NotFound("Affiliate not found".into()))?;
    
    let conv = sqlx::query_as::<_, AffiliateConversion>(
        "INSERT INTO affiliate_conversions (tenant_id, affiliate_id, product_id, lead_id, source_app, commission_amount, commission_rate, cookie_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, tenant_id, affiliate_id, product_id, lead_id, source_app, commission_amount, commission_rate, status, cookie_id, converted_at, paid_at, notes"
    )
    .bind(tenant_id)
    .bind(&req.affiliate_id)
    .bind(req.product_id)
    .bind(req.lead_id)
    .bind(&req.source_app)
    .bind(req.commission_amount)
    .bind(req.commission_rate)
    .bind(&req.cookie_id)
    .fetch_one(&state.pool)
    .await?;
    
    Ok(Json(json!({"data": conv})))
}

pub async fn list_conversions(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    
    let affiliate_id = params.get("affiliate_id");
    
    let conversions = if let Some(aid) = affiliate_id {
        sqlx::query_as::<_, AffiliateConversion>(
            "SELECT id, tenant_id, affiliate_id, product_id, lead_id, source_app, commission_amount, commission_rate, status, cookie_id, converted_at, paid_at, notes
             FROM affiliate_conversions WHERE tenant_id = $1 AND affiliate_id = $2 ORDER BY converted_at DESC"
        )
        .bind(tenant_id)
        .bind(aid)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, AffiliateConversion>(
            "SELECT id, tenant_id, affiliate_id, product_id, lead_id, source_app, commission_amount, commission_rate, status, cookie_id, converted_at, paid_at, notes
             FROM affiliate_conversions WHERE tenant_id = $1 ORDER BY converted_at DESC"
        )
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?
    };
    
    Ok(Json(json!({"data": conversions})))
}
