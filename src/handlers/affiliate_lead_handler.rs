use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use chrono::Utc;

#[derive(Debug, Deserialize)]
pub struct SubmitLeadRequest {
    pub token: String,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub target_app_slug: String,
    pub source: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkSubmitLeadsRequest {
    pub token: String,
    pub target_app_slug: String,
    pub leads: Vec<BulkLeadEntry>,
}

#[derive(Debug, Deserialize)]
pub struct BulkLeadEntry {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ProspectResponse {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub target_app_slug: String,
    pub status: String,
    pub created_at: String,
}

/// Verify the affiliate portal JWT token and return claims
fn verify_affiliate_token(token: &str, state: &AppState) -> Result<(String, String, String), AppError> {
    use jsonwebtoken::{decode, DecodingKey, Validation};
    
    let token_data = decode::<crate::handlers::affiliate_portal_handler::AffiliateClaims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    ).map_err(|_| AppError::Unauthorized("Invalid affiliate token".into()))?;
    
    let claims = token_data.claims;
    Ok((claims.sub, claims.aff_id, claims.tenant_id))
}

/// Submit a single lead/prospect as an affiliate
pub async fn submit_affiliate_lead(
    State(state): State<AppState>,
    Json(req): Json<SubmitLeadRequest>,
) -> AppResult<Json<Value>> {
    let (_user_id, aff_id, tenant_id_str) = verify_affiliate_token(&req.token, &state)?;
    let tenant_id: Uuid = tenant_id_str.parse().map_err(|_| AppError::Internal("Invalid tenant".into()))?;
    
    // Validate: must have email or phone
    if req.email.is_none() && req.phone.is_none() {
        return Err(AppError::BadRequest("Email or phone is required".into()));
    }
    
    // Build tags: combine passed tags with auto-tags
    let mut all_tags: Vec<String> = req.tags.unwrap_or_default();
    all_tags.push(format!("affiliate:{}", aff_id));
    all_tags.push("source:affiliate_submission".to_string());
    if let Some(ref email) = req.email {
        all_tags.push(format!("email:{}", email));
    }
    let tags_json: Value = serde_json::to_value(&all_tags).unwrap_or(json!([]));
    
    // Insert the prospect
    let prospect_id = Uuid::new_v4();
    let now = Utc::now().naive_utc();
    
    sqlx::query(
        "INSERT INTO affiliate_prospects (id, tenant_id, affiliate_id, name, email, phone, company, source, target_app_slug, tags, status, notes, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"
    )
    .bind(prospect_id)
    .bind(tenant_id)
    .bind(&aff_id)
    .bind(&req.name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.company)
    .bind(req.source.as_deref().unwrap_or("affiliate_submission"))
    .bind(&req.target_app_slug)
    .bind(&tags_json)
    .bind("pending")
    .bind(&req.notes)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await?;
    
    // Log the tracking event
    sqlx::query(
        "INSERT INTO affiliate_lead_tracking (tenant_id, affiliate_id, lead_email, source_app, target_app, action, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(tenant_id)
    .bind(&aff_id)
    .bind(&req.email)
    .bind("funnelswift")
    .bind(&req.target_app_slug)
    .bind("submission")
    .bind(json!({"prospect_id": prospect_id.to_string(), "name": &req.name}))
    .execute(&state.pool)
    .await?;
    
    // If email is provided, also update/add to an existing lead in FunnelSwift's leads table
    // with the affiliate tag attached
    if let Some(ref email) = req.email {
        let existing_lead: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM leads WHERE email = $1 AND tenant_id = $2 LIMIT 1"
        )
        .bind(email)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?;
        
        if let Some((lead_id,)) = existing_lead {
            // Update existing lead - append affiliate tags
            sqlx::query(
                "UPDATE leads SET tags = CASE
                    WHEN tags IS NULL OR tags = '[]'::jsonb THEN $1::jsonb
                    ELSE (tags::jsonb || $1::jsonb)::jsonb
                END, updated_at = NOW() WHERE id = $2"
            )
            .bind(json!([format!("affiliate:{}", aff_id), format!("target:{}", req.target_app_slug)]))
            .bind(lead_id)
            .execute(&state.pool)
            .await?;
            
            // Link prospect to this lead
            sqlx::query("UPDATE affiliate_prospects SET linked_lead_id = $1 WHERE id = $2")
                .bind(lead_id)
                .bind(prospect_id)
                .execute(&state.pool)
                .await?;
        } else {
            // Create a new lead in FunnelSwift with the affiliate tagging
            sqlx::query(
                "INSERT INTO leads (id, tenant_id, name, email, phone, company, source, status, tags, notes)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
            )
            .bind(Uuid::new_v4())
            .bind(tenant_id)
            .bind(&req.name)
            .bind(email)
            .bind(&req.phone)
            .bind(&req.company)
            .bind(format!("affiliate_submission:{}", aff_id))
            .bind("new")
            .bind(json!([format!("affiliate:{}", aff_id), format!("target:{}", req.target_app_slug)]))
            .bind(&req.notes)
            .execute(&state.pool)
            .await?;
        }
    }
    
    Ok(Json(json!({
        "success": true,
        "prospect_id": prospect_id.to_string(),
        "affiliate_id": aff_id,
        "target_app": req.target_app_slug,
        "status": "pending"
    })))
}

/// Bulk submit leads (for CSV import / batch scanning)
pub async fn bulk_submit_affiliate_leads(
    State(state): State<AppState>,
    Json(req): Json<BulkSubmitLeadsRequest>,
) -> AppResult<Json<Value>> {
    let (_user_id, aff_id, tenant_id_str) = verify_affiliate_token(&req.token, &state)?;
    let tenant_id: Uuid = tenant_id_str.parse().map_err(|_| AppError::Internal("Invalid tenant".into()))?;
    
    let mut results: Vec<Value> = Vec::new();
    
    for lead in &req.leads {
        if lead.email.is_none() && lead.phone.is_none() {
            results.push(json!({"name": &lead.name, "status": "skipped", "reason": "No email or phone"}));
            continue;
        }
        
        let mut all_tags: Vec<String> = lead.tags.clone().unwrap_or_default();
        all_tags.push(format!("affiliate:{}", aff_id));
        all_tags.push("source:affiliate_submission".to_string());
        if let Some(ref email) = lead.email {
            all_tags.push(format!("email:{}", email));
        }
        let tags_json: Value = serde_json::to_value(&all_tags).unwrap_or(json!([]));
        
        let prospect_id = Uuid::new_v4();
        let now = Utc::now().naive_utc();
        
        sqlx::query(
            "INSERT INTO affiliate_prospects (id, tenant_id, affiliate_id, name, email, phone, company, source, target_app_slug, tags, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
        )
        .bind(prospect_id)
        .bind(tenant_id)
        .bind(&aff_id)
        .bind(&lead.name)
        .bind(&lead.email)
        .bind(&lead.phone)
        .bind(&lead.company)
        .bind("bulk_affiliate_submission")
        .bind(&req.target_app_slug)
        .bind(&tags_json)
        .bind("pending")
        .bind(now)
        .bind(now)
        .execute(&state.pool)
        .await?;
        
        results.push(json!({
            "id": prospect_id.to_string(),
            "name": &lead.name,
            "email": &lead.email,
            "status": "submitted"
        }));
    }
    
    Ok(Json(json!({
        "success": true,
        "submitted": results.len(),
        "affiliate_id": aff_id,
        "target_app": req.target_app_slug,
        "results": results
    })))
}

/// Get the affiliate's submitted leads/prospects
pub async fn list_affiliate_prospects(
    State(state): State<AppState>,
    Json(req): Json<crate::handlers::affiliate_portal_handler::DashboardRequest>,
) -> AppResult<Json<Value>> {
    let (_user_id, aff_id, _tenant_id_str) = verify_affiliate_token(&req.token, &state)?;
    
    let prospects: Vec<ProspectResponse> = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, String, String, chrono::NaiveDateTime)>(
        "SELECT id, name, email, phone, target_app_slug, status, created_at
         FROM affiliate_prospects WHERE affiliate_id = $1 ORDER BY created_at DESC LIMIT 100"
    )
    .bind(&aff_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(id, name, email, phone, target_app, status, created_at)| {
        ProspectResponse {
            id: id.to_string(),
            name,
            email,
            phone,
            target_app_slug: target_app,
            status,
            created_at: created_at.to_string(),
        }
    })
    .collect();
    
    Ok(Json(json!({
        "success": true,
        "prospects": prospects,
        "total": prospects.len()
    })))
}

/// Cross-app API: Check if an email is tracked to any affiliate
/// Called by WorkflowSwift, IncentiveSwift, etc. on signup/upgrade
pub async fn check_affiliate_for_email(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = req.get("email")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("email required".into()))?;
    
    let result: Option<(String, String, f64)> = sqlx::query_as(
        "SELECT a.id, a.name, a.commission_rate
         FROM affiliates a
         JOIN affiliate_prospects ap ON ap.affiliate_id = a.id
         WHERE ap.email = $1 AND a.is_active = true AND a.status = 'active'
         LIMIT 1"
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await?;
    
    if let Some((aff_id, name, rate)) = result {
        Ok(Json(json!({
            "matched": true,
            "affiliate_id": aff_id,
            "affiliate_name": name,
            "commission_rate": rate
        })))
    } else {
        Ok(Json(json!({"matched": false})))
    }
}

/// Cross-app API: Log a lead movement (e.g., signed up for WorkflowSwift, upgraded, etc.)
pub async fn log_lead_movement(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = req.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let target_app = req.get("target_app").and_then(|v| v.as_str()).unwrap_or("");
    let action = req.get("action").and_then(|v| v.as_str()).unwrap_or("signup");
    let api_key = req.get("api_key").and_then(|v| v.as_str());
    
    // Find the affiliate for this email
    let affiliate: Option<(String, String)> = sqlx::query_as(
        "SELECT a.id, a.name FROM affiliates a
         JOIN affiliate_prospects ap ON ap.affiliate_id = a.id
         WHERE ap.email = $1 AND a.is_active = true AND a.status = 'active'
         LIMIT 1"
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await?;
    
    if let Some((aff_id, _name)) = affiliate {
        let tenant: (Uuid,) = sqlx::query_as("SELECT tenant_id FROM affiliates WHERE id = $1")
            .bind(&aff_id)
            .fetch_one(&state.pool)
            .await?;
        
        sqlx::query(
            "INSERT INTO affiliate_lead_tracking (tenant_id, affiliate_id, lead_email, source_app, target_app, action, metadata)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        )
        .bind(tenant.0)
        .bind(&aff_id)
        .bind(email)
        .bind("funnelswift")
        .bind(target_app)
        .bind(action)
        .bind(json!({"api_key": api_key}))
        .execute(&state.pool)
        .await?;
        
        Ok(Json(json!({"logged": true, "affiliate_id": aff_id, "action": action})))
    } else {
        Ok(Json(json!({"logged": false, "reason": "no affiliate match"})))
    }
}

/// Get affiliate's leads stats
pub async fn get_affiliate_leads_stats(
    State(state): State<AppState>,
    Json(req): Json<crate::handlers::affiliate_portal_handler::DashboardRequest>,
) -> AppResult<Json<Value>> {
    let (_user_id, aff_id, _tenant_id_str) = verify_affiliate_token(&req.token, &state)?;
    
    let total_prospects: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM affiliate_prospects WHERE affiliate_id = $1"
    )
    .bind(&aff_id)
    .fetch_one(&state.pool)
    .await?;
    
    let by_app: Vec<(String, i64)> = sqlx::query_as(
        "SELECT target_app_slug, COUNT(*) as cnt FROM affiliate_prospects WHERE affiliate_id = $1 GROUP BY target_app_slug ORDER BY cnt DESC"
    )
    .bind(&aff_id)
    .fetch_all(&state.pool)
    .await?;
    
    let recent_activity: Vec<Value> = sqlx::query_as::<_, (String, String, Option<String>, chrono::NaiveDateTime)>(
        "SELECT action, target_app, lead_email, created_at
         FROM affiliate_lead_tracking WHERE affiliate_id = $1 ORDER BY created_at DESC LIMIT 20"
    )
    .bind(&aff_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(action, target, email, created)| {
        json!({"action": action, "target": target, "email": email, "created_at": created.to_string()})
    })
    .collect();
    
    Ok(Json(json!({
        "total_prospects": total_prospects,
        "by_app": by_app.into_iter().map(|(slug, cnt)| json!({"app": slug, "count": cnt})).collect::<Vec<Value>>(),
        "recent_activity": recent_activity
    })))
}
