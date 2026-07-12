use axum::{extract::State, Json};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::error::{AppError, AppResult};
use crate::models::affiliate_portal::*;
use crate::AppState;
use chrono::Utc;

#[derive(Debug, Serialize, Deserialize)]
pub struct AffiliateClaims {
    pub sub: String,
    pub aff_id: String,
    pub tenant_id: String,
    pub exp: usize,
}

#[derive(Debug, Deserialize)]
pub struct AffiliateLoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct DashboardRequest {
    pub token: String,
}

pub async fn affiliate_signup(
    State(state): State<AppState>,
    Json(req): Json<AffiliateSignupRequest>,
) -> AppResult<Json<Value>> {
    let tenant: (Uuid,) = sqlx::query_as("SELECT id FROM tenants WHERE id != '00000000-0000-0000-0000-000000000001' ORDER BY created_at ASC LIMIT 1")
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::BadRequest("No tenant configured".into()))?;
    let tenant_id = tenant.0;
    
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM affiliate_users WHERE email = $1"
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await?;
    if existing.is_some() {
        return Err(AppError::BadRequest("Email already registered".into()));
    }
    
    let hashed = hash(&req.password, DEFAULT_COST).map_err(|_| AppError::Internal("Failed to hash password".into()))?;
    let aff_id = format!("AFF-{}", Uuid::new_v4().to_string()[..8].to_uppercase());
    let user_id = Uuid::new_v4().to_string();
    let default_commission = req.commission_rate.unwrap_or(10.0);
    
    sqlx::query(
        "INSERT INTO affiliates (id, tenant_id, name, email, commission_rate, status) VALUES ($1, $2, $3, $4, $5, 'active')"
    )
    .bind(&aff_id)
    .bind(tenant_id)
    .bind(format!("{} {}", req.first_name.as_deref().unwrap_or(""), req.last_name.as_deref().unwrap_or("")).trim())
    .bind(&req.email)
    .bind(default_commission)
    .execute(&state.pool)
    .await?;
    
    sqlx::query(
        "INSERT INTO affiliate_users (id, tenant_id, affiliate_id, email, password_hash, first_name, last_name) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(&user_id)
    .bind(tenant_id)
    .bind(&aff_id)
    .bind(&req.email)
    .bind(&hashed)
    .bind(&req.first_name)
    .bind(&req.last_name)
    .execute(&state.pool)
    .await?;
    
    for app in &req.selected_apps {
        sqlx::query(
            "INSERT INTO affiliate_selections (affiliate_user_id, app_slug) VALUES ($1, $2) ON CONFLICT (affiliate_user_id, app_slug) DO NOTHING"
        )
        .bind(&user_id)
        .bind(app)
        .execute(&state.pool)
        .await?;
    }
    
    let code = format!("ref-{}", &Uuid::new_v4().to_string()[..8]);
    sqlx::query(
        "INSERT INTO affiliate_links (tenant_id, affiliate_id, target_url, code) VALUES ($1, $2, $3, $4)"
    )
    .bind(tenant_id)
    .bind(&aff_id)
    .bind("https://app.funnelswift.net")
    .bind(&code)
    .execute(&state.pool)
    .await?;
    
    Ok(Json(json!({"success": true, "affiliate_id": aff_id})))
}

pub async fn affiliate_login(
    State(state): State<AppState>,
    Json(req): Json<AffiliateLoginRequest>,
) -> AppResult<Json<Value>> {
    let user: AffiliateUser = sqlx::query_as(
        "SELECT id, tenant_id, affiliate_id, email, password_hash, first_name, last_name, payout_method, payout_details, min_payout, is_active, created_at, updated_at
         FROM affiliate_users WHERE email = $1"
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;
    
    if !user.is_active {
        return Err(AppError::Unauthorized("Account deactivated".into()));
    }
    
    let valid = verify(&req.password, &user.password_hash).map_err(|_| AppError::Internal("Verify error".into()))?;
    if !valid {
        return Err(AppError::Unauthorized("Invalid credentials".into()));
    }
    
    let token = encode(
        &Header::default(),
        &AffiliateClaims {
            sub: user.id.clone(),
            aff_id: user.affiliate_id.clone(),
            tenant_id: user.tenant_id.to_string(),
            exp: (Utc::now().timestamp() + 86400 * 30) as usize,
        },
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|_| AppError::Internal("Token generation failed".into()))?;
    
    Ok(Json(json!({
        "token": token,
        "user": {
            "id": user.id,
            "affiliate_id": user.affiliate_id,
            "email": user.email,
            "first_name": user.first_name,
            "last_name": user.last_name,
            "payout_method": user.payout_method,
            "min_payout": user.min_payout,
        }
    })))
}

pub async fn affiliate_portal_dashboard(
    State(state): State<AppState>,
    Json(req): Json<DashboardRequest>,
) -> AppResult<Json<Value>> {
    let token_data = decode::<AffiliateClaims>(
        &req.token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| AppError::Unauthorized("Invalid token".into()))?;
    
    let claims = token_data.claims;
    let tenant_id: Uuid = claims.tenant_id.parse().unwrap();
    
    let clicks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_clicks WHERE affiliate_id = $1"
    )
    .bind(&claims.aff_id)
    .fetch_one(&state.pool)
    .await?;
    
    let conversions: Vec<(Option<f64>, String)> = sqlx::query_as(
        "SELECT COALESCE(commission_amount, 0), status FROM affiliate_conversions WHERE affiliate_id = $1 ORDER BY converted_at DESC"
    )
    .bind(&claims.aff_id)
    .fetch_all(&state.pool)
    .await?;
    
    let total_earned: f64 = conversions.iter().map(|(a, _)| a.unwrap_or(0.0)).sum();
    let total_pending: f64 = conversions.iter().filter(|(_, s)| s == "pending").map(|(a, _)| a.unwrap_or(0.0)).sum();
    let total_paid: f64 = conversions.iter().filter(|(_, s)| s == "paid").map(|(a, _)| a.unwrap_or(0.0)).sum();
    let conversion_count = conversions.len() as i64;
    
    let links: Vec<serde_json::Value> = sqlx::query_as::<_, (String, String, bool, String)>(
        "SELECT code, target_url, is_active, created_at::text FROM affiliate_links WHERE affiliate_id = $1 ORDER BY created_at DESC"
    )
    .bind(&claims.aff_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(c, u, a, d)| json!({"code": c, "target_url": u, "is_active": a, "created_at": d}))
    .collect();
    
    let selections: Vec<String> = sqlx::query_scalar(
        "SELECT app_slug FROM affiliate_selections WHERE affiliate_user_id = $1 AND is_active = true"
    )
    .bind(&claims.sub)
    .fetch_all(&state.pool)
    .await?;
    
    let payouts: Vec<serde_json::Value> = sqlx::query_as::<_, (f64, String, Option<String>, String)>(
        "SELECT amount, status, method, paid_at::text FROM affiliate_payouts WHERE affiliate_id = $1 ORDER BY created_at DESC"
    )
    .bind(&claims.aff_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(a, s, m, p)| json!({"amount": a, "status": s, "method": m, "paid_at": p}))
    .collect();
    
    Ok(Json(json!({
        "stats": {
            "total_clicks": clicks,
            "total_conversions": conversion_count,
            "total_earned": total_earned,
            "total_pending": total_pending,
            "total_paid": total_paid,
            "conversion_rate": if clicks > 0 { ((conversion_count as f64 / clicks as f64) * 100.0 * 100.0).round() / 100.0 } else { 0.0 }
        },
        "links": links,
        "selections": selections,
        "payouts": payouts
    })))
}
