use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Html,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use std::collections::HashMap;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunnelStep {
    pub order: i32,
    pub card_slug: String,
    pub button_label: String,
    pub button_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFunnelInput {
    pub name: String,
    pub slug: String,
    pub steps: Vec<FunnelStep>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFunnelInput {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub steps: Option<Vec<FunnelStep>>,
    pub is_active: Option<bool>,
}

// ── CRUD ──

pub async fn list_funnels(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows = sqlx::query_as::<_, (Uuid, String, String, Value, bool, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, name, slug, steps, is_active, created_at FROM funnels WHERE tenant_id = $1 ORDER BY created_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let funnels: Vec<Value> = rows.iter().map(|r| {
        let steps: Vec<FunnelStep> = serde_json::from_value(r.3.clone()).unwrap_or_default();
        let step_count = steps.len();
        json!({
            "id": r.0.to_string(), "name": r.1, "slug": r.2,
            "steps": steps, "step_count": step_count,
            "is_active": r.4, "created_at": r.5.to_rfc3339(),
        })
    }).collect();

    Ok(Json(json!({"funnels": funnels, "total": funnels.len()})))
}

pub async fn create_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(input): Json<CreateFunnelInput>,
) -> AppResult<(axum::http::StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let id = Uuid::new_v4();
    let steps_json = serde_json::to_value(&input.steps).unwrap_or(json!([]));
    let now = chrono::Utc::now();

    sqlx::query(
        "INSERT INTO funnels (id, tenant_id, name, slug, steps, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(id).bind(tenant_id).bind(&input.name).bind(&input.slug).bind(&steps_json).bind(now).bind(now)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.constraint().is_some() {
                return AppError::Conflict("A funnel with this slug already exists".into());
            }
        }
        AppError::Internal(e.to_string())
    })?;

    let subdomain: Option<String> = sqlx::query_scalar("SELECT subdomain FROM tenants WHERE id = $1")
        .bind(tenant_id).fetch_optional(&state.pool).await.ok().flatten();

    let public_url = if let Some(sd) = subdomain {
        format!("https://{}.kntcrd.com/funnel/{}", sd, input.slug)
    } else {
        format!("https://funnelswift.net/funnel/{}", input.slug)
    };

    Ok((axum::http::StatusCode::CREATED, Json(json!({
        "id": id.to_string(), "name": input.name, "slug": input.slug,
        "steps": input.steps, "public_url": public_url, "message": "Funnel created"
    }))))
}

pub async fn get_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let row = sqlx::query_as::<_, (Uuid, String, String, Value, bool, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, name, slug, steps, is_active, created_at, updated_at FROM funnels WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id).bind(tenant_id)
    .fetch_optional(&state.pool).await?
    .ok_or_else(|| AppError::NotFound("Funnel not found".into()))?;

    let steps: Vec<FunnelStep> = serde_json::from_value(row.3).unwrap_or_default();
    Ok(Json(json!({
        "id": row.0.to_string(), "name": row.1, "slug": row.2,
        "steps": steps, "is_active": row.4,
        "created_at": row.5.to_rfc3339(), "updated_at": row.6.to_rfc3339(),
    })))
}

pub async fn update_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateFunnelInput>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_as::<_, (String, String, Value, bool)>(
        "SELECT name, slug, steps, is_active FROM funnels WHERE id = $1 AND tenant_id = $2"
    )
    .bind(id).bind(tenant_id)
    .fetch_optional(&state.pool).await?
    .ok_or_else(|| AppError::NotFound("Funnel not found".into()))?;

    let name = input.name.unwrap_or(existing.0);
    let slug = input.slug.unwrap_or(existing.1);
    let steps = input.steps.map(|s| serde_json::to_value(s).unwrap_or(existing.2.clone())).unwrap_or(existing.2);
    let is_active = input.is_active.unwrap_or(existing.3);

    sqlx::query("UPDATE funnels SET name = $1, slug = $2, steps = $3, is_active = $4, updated_at = NOW() WHERE id = $5 AND tenant_id = $6")
        .bind(&name).bind(&slug).bind(&steps).bind(is_active).bind(id).bind(tenant_id)
        .execute(&state.pool).await?;

    Ok(Json(json!({"message": "Funnel updated", "id": id.to_string()})))
}

pub async fn delete_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM funnels WHERE id = $1 AND tenant_id = $2")
        .bind(id).bind(tenant_id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Funnel deleted"})))
}

// ── Public Render — Redirects to first card in the funnel ──

pub async fn render_funnel(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Html<String>, AppError> {
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
    let tenant_slug = headers.get("x-tenant-slug").and_then(|v| v.to_str().ok());

    // Find funnel
    let (funnel_id, tenant_id, funnel_name, steps_json) = if let Some(ts) = tenant_slug {
        sqlx::query_as::<_, (Uuid, Uuid, String, Value)>(
            "SELECT f.id, f.tenant_id, f.name, f.steps FROM funnels f JOIN tenants t ON f.tenant_id = t.id WHERE f.slug = $1 AND t.subdomain = $2 AND f.is_active = true"
        )
        .bind(&slug).bind(ts)
        .fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Funnel not found".into()))?
    } else {
        sqlx::query_as::<_, (Uuid, Uuid, String, Value)>(
            "SELECT id, tenant_id, name, steps FROM funnels WHERE slug = $1 AND is_active = true"
        )
        .bind(&slug)
        .fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Funnel not found".into()))?
    };

    let steps: Vec<FunnelStep> = serde_json::from_value(steps_json).unwrap_or_default();
    if steps.is_empty() {
        return Err(AppError::NotFound("Funnel has no steps".into()));
    }

    // Get current step from query param
    let step_idx: usize = params.get("step").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1).min(steps.len());
    let current_step = &steps[step_idx - 1];
    let is_last = step_idx >= steps.len();
    let progress_pct = (step_idx * 100) / steps.len();

    // Fetch the card for this step
    let card = sqlx::query_as::<_, crate::handlers::kinetic_handler::KineticCard>(
        "SELECT * FROM kinetic_cards WHERE slug = $1 AND tenant_id = $2 AND is_active = true"
    )
    .bind(&current_step.card_slug)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Card '{}' not found", current_step.card_slug)))?;

    // Build the card HTML using the same render pipeline as render_card
    let blocks = crate::handlers::kinetic_handler::blocks_from_legacy_card(&card);
    let limits = crate::handlers::kinetic_handler::get_user_limits(&state.pool, card.tenant_id).await;
    let card_type_label = blocks.iter().find_map(|b| match b {
        crate::templates::LayoutBlock::BioLink { .. } => Some("Bio Link"),
        crate::templates::LayoutBlock::BusinessCard { .. } => Some("Digital Business Card"),
        crate::templates::LayoutBlock::MiniFunnel { .. } => Some("Mini Funnel"),
        crate::templates::LayoutBlock::Hero { .. } => Some("Mini Page"),
        _ => None,
    }).unwrap_or("Kinetic Card");
    let cta_label = limits.cta_text.replace("{{type}}", card_type_label);

    let tmpl = crate::templates::PageTemplate {
        tenant_name: &card.title,
        page_title: &card.title,
        meta_description: card.meta_description.as_deref().unwrap_or(""),
        primary_color: &card.bg_color,
        accent_color: &card.accent_color,
        custom_css: "",
        slug: &card.slug,
        logo_url: card.logo_url.as_deref(),
        blocks,
        modal_form_title: "Get Started",
        modal_button_text: "Submit",
        modal_placeholder: "Enter your email",
        modal_fields: vec![],
        show_branding: limits.show_branding,
        page_password_hash: None,
        page_consent_required: false,
        affiliate_code: None,
        cta_label: &cta_label,
        is_dark: crate::handlers::kinetic_handler::is_dark_color(&card.bg_color),
        theme: card.theme.as_deref(),
    };

    let card_html = askama::Template::render(&tmpl).map_err(|e| {
        tracing::error!("Funnel Askama render error: {e}");
        AppError::Internal(format!("Render error: {e}"))
    })?;

    // Build funnel navigation + progress bar
    let nav_html = format!(
        r#"<div style="position:fixed;top:0;left:0;right:0;z-index:1000;background:rgba(15,23,42,.95);backdrop-filter:blur(10px);border-bottom:1px solid rgba(148,163,184,.1);padding:10px 16px">
            <div style="max-width:600px;margin:0 auto;display:flex;align-items:center;gap:10px">
                <div style="color:#94a3b8;font-size:11px;white-space:nowrap">Step {}/{} </div>
                <div style="flex:1;height:3px;background:#334155;border-radius:2px;overflow:hidden">
                    <div style="height:100%;width:{}%;background:linear-gradient(90deg,var(--primary),var(--accent));border-radius:2px;transition:width .3s"></div>
                </div>
                <a href="/funnel/{}?step={}" style="background:{};color:#fff;padding:6px 14px;border-radius:6px;font-size:12px;font-weight:600;text-decoration:none;white-space:nowrap">{}</a>
            </div>
        </div>"#,
        step_idx, steps.len(), progress_pct,
        slug, if is_last { step_idx } else { step_idx + 1 },
        "var(--accent)",
        if is_last { &current_step.button_label } else { &current_step.button_label }
    );

    // Inject nav bar before the card content
    let full_html = card_html.replace("<body", &format!("<body style="padding-top:48px"{0}", if nav_html.is_empty() { "" } else { "" }));
    let full_html = full_html.replace("</body>", &format!("{}</body>", nav_html));

    // Replace the page footer CTA with the step's button URL
    let button_url = if let Some(ref external_url) = current_step.button_url {
        if !external_url.is_empty() { external_url.clone() } else { "#".to_string() }
    } else { "#".to_string() };
    let full_html = full_html.replace(
        r#"<a href="https://funnelswift.net/kinetic"#,
        &format!(r#"<a href="{}" "#, button_url)
    );

    Ok(Html(full_html))
}
