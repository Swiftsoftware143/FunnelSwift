use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
use crate::error::AppResult;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ConversionWebhook {
    pub affiliate_id: Option<String>,
    pub cookie_id: Option<String>,
    pub source_app: String,
    pub event: String,
    pub lead_id: Option<String>,
    pub lead_email: Option<String>,
    pub product_id: Option<String>,
    pub amount: Option<f64>,
    pub metadata: Option<Value>,
}

pub async fn handle_conversion_webhook(
    State(state): State<AppState>,
    Json(req): Json<ConversionWebhook>,
) -> AppResult<Json<Value>> {
    // Try to find affiliate by cookie_id first, then by affiliate_id
    let affiliate_id = if let Some(ref aid) = req.affiliate_id {
        Some(aid.clone())
    } else if let Some(ref cid) = req.cookie_id {
        // Look up the most recent click with this cookie_id
        let click: Option<(String,)> = sqlx::query_as(
            "SELECT affiliate_id FROM affiliate_clicks WHERE cookie_id = $1 ORDER BY clicked_at DESC LIMIT 1"
        )
        .bind(cid)
        .fetch_optional(&state.pool)
        .await?;
        click.map(|(a,)| a)
    } else {
        None
    };

    let aff_id = match affiliate_id {
        Some(id) => id,
        None => return Ok(Json(json!({"status": "skipped", "reason": "no affiliate found"}))),
    };

    // Get tenant from affiliate
    let tenant: Option<(Uuid,)> = sqlx::query_as(
        "SELECT tenant_id FROM affiliates WHERE id = $1"
    )
    .bind(&aff_id)
    .fetch_optional(&state.pool)
    .await?;

    let (tenant_id,) = match tenant {
        Some(t) => t,
        None => return Ok(Json(json!({"status": "skipped", "reason": "affiliate not found"}))),
    };

    // Get default commission rate for this affiliate
    let commission_rate: Option<f64> = sqlx::query_scalar(
        "SELECT commission_rate FROM affiliates WHERE id = $1"
    )
    .bind(&aff_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    let rate = commission_rate.unwrap_or(10.0);
    let amount = req.amount.unwrap_or(0.0);
    let commission = amount * (rate / 100.0);

    // Parse lead_id to UUID if present
    let lead_uuid = req.lead_id.as_ref().and_then(|id| Uuid::parse_str(id).ok());

    // Parse product_id to UUID if present
    let product_uuid = req.product_id.as_ref().and_then(|id| Uuid::parse_str(id).ok());

    // Record the conversion
    sqlx::query(
        "INSERT INTO affiliate_conversions (tenant_id, affiliate_id, product_id, lead_id, source_app, commission_amount, commission_rate, cookie_id, notes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
    )
    .bind(tenant_id)
    .bind(&aff_id)
    .bind(product_uuid)
    .bind(lead_uuid)
    .bind(&req.source_app)
    .bind(commission)
    .bind(rate)
    .bind(&req.cookie_id)
    .bind(req.metadata.map(|m| m.to_string()))
    .execute(&state.pool)
    .await?;

    // Update the affiliate click with conversion info
    if let Some(ref cid) = req.cookie_id {
        sqlx::query(
            "UPDATE affiliate_clicks SET landing_page = 'converted' WHERE cookie_id = $1 AND landing_page IS NULL"
        )
        .bind(cid)
        .execute(&state.pool)
        .await?;
    }

    Ok(Json(json!({"status": "recorded", "affiliate_id": aff_id, "commission": commission})))
}

// Track conversion from lead form (lead capture embedded in any app)
#[derive(Debug, Deserialize)]
pub struct LeadConversionRequest {
    pub cookie_id: Option<String>,
    pub aff_ref: Option<String>,
    pub source_app: String,
    pub lead_email: String,
    pub lead_name: Option<String>,
    pub product_slug: Option<String>,
}

pub async fn track_lead_conversion(
    State(state): State<AppState>,
    Json(req): Json<LeadConversionRequest>,
) -> AppResult<Json<Value>> {
    // Determine affiliate
    let affiliate_id = if let Some(ref cid) = req.cookie_id {
        let click: Option<(String,)> = sqlx::query_as(
            "SELECT affiliate_id FROM affiliate_clicks WHERE cookie_id = $1 ORDER BY clicked_at DESC LIMIT 1"
        )
        .bind(cid)
        .fetch_optional(&state.pool)
        .await?;
        click.map(|(a,)| a)
    } else if let Some(ref code) = req.aff_ref {
        let link: Option<(String,)> = sqlx::query_as(
            "SELECT affiliate_id FROM affiliate_links WHERE code = $1 AND is_active = true"
        )
        .bind(code)
        .fetch_optional(&state.pool)
        .await?;
        link.map(|(a,)| a)
    } else {
        None
    };

    let aff_id = match affiliate_id {
        Some(id) => id,
        None => return Ok(Json(json!({"status": "notrack", "reason": "no affiliate cookie"}))),
    };

    // Create a new lead reference
    // (the actual lead is created by the app, we just track the conversion)
    let tenant: (Uuid,) = sqlx::query_as(
        "SELECT tenant_id FROM affiliates WHERE id = $1"
    )
    .bind(&aff_id)
    .fetch_one(&state.pool)
    .await?;

    let commission_rate: Option<f64> = sqlx::query_scalar(
        "SELECT commission_rate FROM affiliates WHERE id = $1"
    )
    .bind(&aff_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let rate = commission_rate.unwrap_or(10.0);

    sqlx::query(
        "INSERT INTO affiliate_conversions (tenant_id, affiliate_id, source_app, commission_rate, cookie_id, notes)
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(tenant.0)
    .bind(&aff_id)
    .bind(&req.source_app)
    .bind(rate)
    .bind(&req.cookie_id)
    .bind(json!({"email": req.lead_email, "name": req.lead_name}).to_string())
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"status": "recorded", "affiliate_id": aff_id})))
}
