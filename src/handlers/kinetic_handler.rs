//! Kinetic Bio-Links Module — SSR pages via Askama with dynamic LayoutBlock rendering.
//!
//! Powering both:
//!   1. BIO-LINK LAYOUT (Linktree-style) — avatar, bio, pill buttons, social icons
//!   2. DYNAMIC MINI-PAGE (section-based landing page) — hero, features grid, lead form
//!
//! Both share the same /k/:slug endpoint; the layout_blocks JSONB column on kinetic_cards
//! determines which blocks render.

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::templates::{self, LayoutBlock, PageTemplate};
use std::borrow::Cow;

// ──────────────────────────────────────────────
// STRUCTS
// ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct KineticCard {
    pub id: Uuid,
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub slug: String,
    pub title: String,
    pub tagline: Option<String>,
    pub bio: Option<String>,
    pub meta_description: Option<String>,
    pub logo_url: Option<String>,
    pub avatar_url: Option<String>,
    pub template_type: String,
    pub video_provider: Option<String>,
    pub video_id: Option<String>,
    pub bg_color: String,
    pub text_color: String,
    pub accent_color: String,
    pub button_bg_color: String,
    pub button_text_color: String,
    pub instagram_url: Option<String>,
    pub facebook_url: Option<String>,
    pub twitter_url: Option<String>,
    pub youtube_url: Option<String>,
    pub linkedin_url: Option<String>,
    pub tiktok_url: Option<String>,
    pub layout_blocks: Option<serde_json::Value>,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct KineticButton {
    pub id: Uuid,
    pub card_id: Uuid,
    pub label: String,
    pub sort_order: i32,
    pub action_type: String,
    pub destination_url: Option<String>,
    pub target_tag_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct KineticSource {
    pub id: Uuid,
    pub card_id: Uuid,
    pub source_param: String,
    pub target_tag_id: Uuid,
    pub label: Option<String>,
}

// ──────────────────────────────────────────────
// INPUT TYPES
// ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateCardInput {
    pub slug: String,
    pub title: String,
    pub tagline: Option<String>,
    pub bio: Option<String>,
    pub meta_description: Option<String>,
    pub logo_url: Option<String>,
    pub avatar_url: Option<String>,
    pub template_type: Option<String>,
    pub video_provider: Option<String>,
    pub video_id: Option<String>,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub accent_color: Option<String>,
    pub button_bg_color: Option<String>,
    pub button_text_color: Option<String>,
    pub instagram_url: Option<String>,
    pub facebook_url: Option<String>,
    pub twitter_url: Option<String>,
    pub youtube_url: Option<String>,
    pub linkedin_url: Option<String>,
    pub tiktok_url: Option<String>,
    /// Optional pre-built layout blocks (JSON array). If omitted, a default BioLink block is auto-generated.
    pub layout_blocks: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCardInput {
    pub title: Option<String>,
    pub tagline: Option<String>,
    pub bio: Option<String>,
    pub meta_description: Option<String>,
    pub logo_url: Option<String>,
    pub avatar_url: Option<String>,
    pub slug: Option<String>,
    pub template_type: Option<String>,
    pub video_provider: Option<String>,
    pub video_id: Option<String>,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub accent_color: Option<String>,
    pub button_bg_color: Option<String>,
    pub button_text_color: Option<String>,
    pub instagram_url: Option<String>,
    pub facebook_url: Option<String>,
    pub twitter_url: Option<String>,
    pub youtube_url: Option<String>,
    pub linkedin_url: Option<String>,
    pub tiktok_url: Option<String>,
    pub layout_blocks: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateButtonInput {
    pub card_id: Uuid,
    pub label: String,
    pub sort_order: Option<i32>,
    pub action_type: String,
    pub destination_url: Option<String>,
    pub target_tag_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSourceInput {
    pub card_id: Uuid,
    pub source_param: String,
    pub target_tag_id: Uuid,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CardQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct TrackClickQuery {
    pub button_id: Uuid,
    pub card_id: Uuid,
    pub src: Option<String>,
}

/// Lead form submission from the SSR page
#[derive(Debug, Deserialize)]
pub struct LeadFormSubmission {
    pub email: Option<String>,
    pub name: Option<String>,
    pub phone: Option<String>,
}

// ──────────────────────────────────────────────
// TIER LIMITS
// ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlanLimits {
    pub max_kinetic_cards: i64,
    pub allow_custom_colors: bool,
    pub allow_videos: bool,
    pub allow_source_tracking: bool,
    pub allow_minipage_layout: bool,
    pub show_branding: bool,  // false = "Powered by FunnelSwift Kinetic" always shown
    pub allow_minifunnel: bool,
    pub cta_text: String,
}

pub async fn get_user_limits(pool: &PgPool, tenant_id: Uuid) -> PlanLimits {
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

    fn get_int(f: &serde_json::Value, key: &str, default: i64) -> i64 {
        f.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
    }
    fn get_bool(f: &serde_json::Value, key: &str, default: bool) -> bool {
        f.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }
    fn get_str(f: &serde_json::Value, key: &str, default: &str) -> String {
        f.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or(default.to_string())
    }

    let cta_text = get_str(&f, "kinetic_cta_text", "Claim Your {{type}}");

    PlanLimits {
        max_kinetic_cards: get_int(&f, "max_kinetic_cards", 1),
        allow_custom_colors: get_bool(&f, "kinetic_custom_colors", false),
        allow_videos: get_bool(&f, "kinetic_video", false),
        allow_source_tracking: get_bool(&f, "kinetic_source_tracking", false),
        allow_minipage_layout: get_bool(&f, "kinetic_minipage", false),
        show_branding: get_bool(&f, "kinetic_branding", true),  // default: show branding
        allow_minifunnel: get_bool(&f, "kinetic_minifunnel", true),
        cta_text,
    }
}

pub async fn enforce_card_limit(pool: &PgPool, tenant_id: Uuid) -> AppResult<()> {
    let limits = get_user_limits(pool, tenant_id).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1 AND is_active = true"
    )
    .bind(tenant_id).fetch_one(pool).await.unwrap_or(0);

    if count >= limits.max_kinetic_cards {
        return Err(AppError::Forbidden(format!(
            "Plan limit: max {} kinetic cards. Upgrade your plan.", limits.max_kinetic_cards
        )));
    }
    Ok(())
}

pub async fn enforce_minipage_access(pool: &PgPool, tenant_id: Uuid) -> AppResult<()> {
    let limits = get_user_limits(pool, tenant_id).await;
    if !limits.allow_minipage_layout {
        return Err(AppError::Forbidden(
            "Dynamic mini-pages (hero/features/leadform blocks) require at least a Growth plan.".into()
        ));
    }
    Ok(())
}

/// Check if the tenant is allowed to create/update cards with MiniFunnel blocks
pub async fn enforce_minifunnel_access(pool: &PgPool, tenant_id: Uuid, blocks: &serde_json::Value) -> AppResult<()> {
    let limits = get_user_limits(pool, tenant_id).await;
    if !limits.allow_minifunnel {
        let has_minifunnel = blocks.as_array().map_or(false, |arr| {
            arr.iter().any(|b| {
                b.get("type").and_then(|t| t.as_str()) == Some("minifunnel")
            })
        });
        if has_minifunnel {
            return Err(AppError::Forbidden(
                "Mini Funnels require at least a Growth plan.".into()
            ));
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────
// EVENT TRACKING
// ──────────────────────────────────────────────

fn record_event(pool: &PgPool, user_id: Uuid, tenant_id: Uuid, card_id: Uuid, event_type: &str, button_id: Option<Uuid>, source_param: Option<String>, ip_hash: Option<String>, source_label: Option<String>) {
    let pool = pool.clone();
    let event = event_type.to_string();
    tokio::spawn(async move {
        let _ = sqlx::query(
            "INSERT INTO lead_events (user_id, tenant_id, card_id, button_id, event_type, source_param, source_label, ip_hash) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        ).bind(user_id).bind(tenant_id).bind(card_id).bind(button_id)
         .bind(&event).bind(&source_param).bind(&source_label).bind(&ip_hash)
         .execute(&pool).await;
    });
}

pub async fn resolve_source_tag(pool: &PgPool, card_id: Uuid, source_param: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT target_tag_id FROM kinetic_sources WHERE card_id = $1 AND source_param = $2")
        .bind(card_id).bind(source_param)
        .fetch_optional(pool).await.ok().flatten()
}

/// Build a default LayoutBlock::BioLink from legacy kinetic_cards columns
fn blocks_from_legacy_card(card: &KineticCard) -> Vec<LayoutBlock> {
    let mut buttons: Vec<templates::BioButton> = Vec::new();
    let mut social_links = Vec::new();
    if let Some(ref u) = card.instagram_url { social_links.push(templates::SocialLink { icon: "📸".into(), url: u.clone() }); }
    if let Some(ref u) = card.facebook_url { social_links.push(templates::SocialLink { icon: "👍".into(), url: u.clone() }); }
    if let Some(ref u) = card.twitter_url { social_links.push(templates::SocialLink { icon: "🐦".into(), url: u.clone() }); }
    if let Some(ref u) = card.youtube_url { social_links.push(templates::SocialLink { icon: "▶️".into(), url: u.clone() }); }
    if let Some(ref u) = card.linkedin_url { social_links.push(templates::SocialLink { icon: "💼".into(), url: u.clone() }); }
    if let Some(ref u) = card.tiktok_url { social_links.push(templates::SocialLink { icon: "🎵".into(), url: u.clone() }); }

    vec![LayoutBlock::BioLink {
        avatar_url: card.avatar_url.clone(),
        video_url: None,
        bio: card.bio.clone().unwrap_or_default(),
        buttons: Vec::new(), // populated at render time
        social_links,
    }]
}

// ──────────────────────────────────────────────
// PUBLIC SSR ENDPOINTS
// ──────────────────────────────────────────────

/// GET /k/:slug — Render bio-link or mini-page from layout_blocks
pub async fn render_card(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let card = sqlx::query_as::<_, KineticCard>(
        "SELECT * FROM kinetic_cards WHERE slug = $1 AND is_active = true"
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Bio-link page not found".to_string()))?;

    // Source tracking
    let src = params.get("src").or(params.get("source")).cloned();
    let ip_hash = params.get("_hash").cloned();
    // Resolve source label from kinetic_sources
    let source_label: Option<String> = if let Some(ref s) = src {
        sqlx::query_scalar("SELECT label FROM kinetic_sources WHERE card_id = $1 AND source_param = $2")
            .bind(card.id).bind(s)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    record_event(&state.pool, card.user_id, card.tenant_id, card.id, "page_view", None, src.clone(), ip_hash, source_label);

    if let Some(ref s) = src {
        resolve_source_tag(&state.pool, card.id, s).await;
    }

    // Resolve layout blocks from JSONB or build from legacy columns
    let blocks: Vec<LayoutBlock> = if let Some(ref blocks_json) = card.layout_blocks {
        if let Ok(b) = serde_json::from_value(blocks_json.clone()) {
            b
        } else {
            // Fallback to legacy
            blocks_from_legacy_card(&card)
        }
    } else {
        blocks_from_legacy_card(&card)
    };

    render_card_html(&state, &card, &blocks, &params, &src).await
}



async fn render_card_html(
    state: &AppState,
    card: &KineticCard,
    blocks: &[LayoutBlock],
    params: &HashMap<String, String>,
    src: &Option<String>,
) -> Result<Html<String>, AppError> {
    // Inject buttons into BioLink blocks
    let db_buttons = sqlx::query_as::<_, KineticButton>(
        "SELECT * FROM kinetic_buttons WHERE card_id = $1 ORDER BY sort_order ASC"
    )
    .bind(card.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let btn_list_for_card: Vec<templates::BioButton> = db_buttons.iter().map(|b| {
        templates::BioButton {
            label: b.label.clone(),
            url: format!("/track/click?button_id={}&card_id={}", b.id, b.card_id),
        }
    }).collect();

    let blocks: Vec<LayoutBlock> = blocks.iter().map(|block| match block {
        LayoutBlock::BioLink { avatar_url, video_url, bio, social_links, .. } => {
            LayoutBlock::BioLink { avatar_url: avatar_url.clone(), video_url: video_url.clone(), bio: bio.clone(), buttons: btn_list_for_card.clone(), social_links: social_links.clone() }
        }
        LayoutBlock::BusinessCard { name, title, company, company_logo_url, avatar_url, catchphrase, phone, email, website, social_links, .. } => {
            LayoutBlock::BusinessCard { name: name.clone(), title: title.clone(), company: company.clone(), company_logo_url: company_logo_url.clone(), avatar_url: avatar_url.clone(), catchphrase: catchphrase.clone(), phone: phone.clone(), email: email.clone(), website: website.clone(), buttons: btn_list_for_card.clone(), social_links: social_links.clone() }
        }
        other => other.clone(),
    }).collect::<Vec<_>>();

    // Build template context — extract lead form fields for the modal
    let (modal_form_title, modal_button_text, modal_placeholder, modal_fields) = blocks.iter().find_map(|b| {
        if let LayoutBlock::LeadForm { form_title, button_text, placeholder, fields } = b {
            Some((form_title.clone(), button_text.clone(), placeholder.clone(), fields.clone()))
        } else {
            None
        }
    }).unwrap_or_else(|| ("Get Started".into(), "Submit".into(), "your@email.com".into(), vec![]));

    // Fetch tenant affiliate code for branding footer
    let page_password_hash_str: Option<String> = sqlx::query_scalar(
        "SELECT password_hash FROM kinetic_cards WHERE id = $1"
    )
    .bind(card.id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let affiliate_code_str: Option<String> = sqlx::query_scalar(
        "SELECT affiliate_code FROM tenants WHERE id = $1"
    )
    .bind(card.tenant_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();

    // Detect card type from blocks for CTA label
    let limits = get_user_limits(&state.pool, card.tenant_id).await;
    let card_type_label = blocks.iter().find_map(|b| match b {
        LayoutBlock::BioLink { .. } | LayoutBlock::BusinessCard { .. } => Some("Bio Link"),
        LayoutBlock::Hero { .. } | LayoutBlock::Features { .. } => Some("Mini Page"),
        LayoutBlock::MiniFunnel { .. } => Some("Mini Funnel"),
        _ => None,
    }).unwrap_or("Kinetic Card");
    let cta_label = limits.cta_text.replace("{{type}}", card_type_label).replace("{type}", card_type_label);

    let tmpl = PageTemplate {
        tenant_name: &card.title,
        page_title: &card.title,
        meta_description: card.meta_description.as_deref().unwrap_or(""),
        primary_color: &card.bg_color,
        accent_color: &card.accent_color,
        custom_css: "",
        slug: &card.slug,
        logo_url: card.logo_url.as_deref(),
        blocks,
        modal_form_title: &modal_form_title,
        modal_button_text: &modal_button_text,
        modal_placeholder: &modal_placeholder,
        modal_fields,
        show_branding: limits.show_branding,
 page_password_hash: page_password_hash_str.as_deref(),
 page_consent_required: false,
        affiliate_code: affiliate_code_str.as_deref(),
        cta_label: &cta_label,
    };

    let html = askama::Template::render(&tmpl).map_err(|e| {
        tracing::error!("Askama render error: {e}");
        AppError::Internal("Template rendering failed".into())
    })?;

    Ok(Html(html))
}
/// GET /track/click — Track button click and redirect
pub async fn track_click(
    State(state): State<AppState>,
    Query(params): Query<TrackClickQuery>,
) -> Result<Redirect, AppError> {
    let button = sqlx::query_as::<_, KineticButton>(
        "SELECT * FROM kinetic_buttons WHERE id = $1 AND card_id = $2"
    )
    .bind(params.button_id).bind(params.card_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Button not found".to_string()))?;

    let card: KineticCard = sqlx::query_as("SELECT * FROM kinetic_cards WHERE id = $1")
        .bind(params.card_id)
        .fetch_one(&state.pool)
        .await?;

    record_event(&state.pool, card.user_id, card.tenant_id, params.card_id, "button_click", Some(params.button_id), params.src, None, None);

    let destination = button.destination_url.unwrap_or_default();

    // If this button has a target_tag, fire the tag → routing pipeline
    if let Some(tag_id) = button.target_tag_id {
        // The tag-to-lead association happens when the lead submits the form
        // For now, we store the tag intent on the lead_events record
        // This the submit_lead handler can resolve later
        tracing::info!("Button click with target_tag_id={}", tag_id);
    }

    Ok(Redirect::to(&destination))
}

/// POST /k/:slug/lead — Handle lead form submissions from SSR pages
pub async fn submit_lead(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<LeadFormSubmission>,
) -> Result<Json<Value>, AppError> {
    let card = sqlx::query_as::<_, KineticCard>(
        "SELECT * FROM kinetic_cards WHERE slug = $1 AND is_active = true"
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Page not found".to_string()))?;

    // Resolve tag from source mapping if src param is present
    // We capture the ?src= param from the page URL lead came from
    // Source info can come via the referrer or query — we use the most recent
    // lead_events page_view for this card to resolve the source
    let resolved_tag_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT ks.target_tag_id FROM lead_events le
           JOIN kinetic_sources ks ON ks.card_id = le.card_id AND ks.source_param = le.source_param
           WHERE le.card_id = $1 AND le.event_type = 'page_view'
           ORDER BY le.created_at DESC LIMIT 1"#
    )
    .bind(card.id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();

    // Also check if there's a recent button click with target_tag_id
    let button_tag_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT kb.target_tag_id FROM lead_events le
           JOIN kinetic_buttons kb ON kb.id = le.button_id
           WHERE le.card_id = $1 AND le.event_type = 'button_click' AND kb.target_tag_id IS NOT NULL
           ORDER BY le.created_at DESC LIMIT 1"#
    )
    .bind(card.id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();

    let tag_id = button_tag_id.or(resolved_tag_id);

    // Resolve tag name for JSONB storage
    let tag_name: Option<String> = if let Some(tid) = tag_id {
        sqlx::query_scalar("SELECT name FROM tags WHERE id = $1")
            .bind(tid)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    // Build tags JSONB — if we have a tag, apply it
    let tags_json = if let Some(ref name) = tag_name {
        serde_json::json!([name])
    } else {
        serde_json::Value::Null
    };

    // Create a lead record in the leads table with the tag applied
    sqlx::query(
        r#"INSERT INTO leads (tenant_id, first_name, email, phone, source, stage, tags)
           VALUES ($1, $2, $3, $4, $5, 'New', $6)"#
    )
    .bind(card.tenant_id)
    .bind(&body.name).bind(&body.email).bind(&body.phone)
    .bind(&format!("kinetic:{}", slug))
    .bind(&tags_json)
    .execute(&state.pool)
    .await?;

    record_event(&state.pool, card.user_id, card.tenant_id, card.id, "form_submit", None, None, None, None);

    // If a tag was applied, log the routing intent
    if let Some(name) = tag_name {
        tracing::info!("Lead tagged '{}' from kinetic card '{}' — routing will process", name, slug);
    }

    Ok(Json(json!({ "status": "success", "message": "Thank you! We'll be in touch." })))
}

// ──────────────────────────────────────────────
// CRUD — Cards
// ──────────────────────────────────────────────

pub async fn list_cards(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<CardQuery>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).min(100).max(1);
    let offset = (page - 1) * per_page;

    let cards = sqlx::query_as::<_, KineticCard>(
        "SELECT * FROM kinetic_cards WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    ).bind(tenant_id).bind(per_page).bind(offset).fetch_all(&state.pool).await?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1")
        .bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);

    Ok(Json(json!({"cards": cards, "total": total, "page": page, "per_page": per_page})))
}

pub async fn create_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateCardInput>,
) -> AppResult<Json<Value>> {
    let user_id: Uuid = auth.user_id.parse().map_err(|_| AppError::BadRequest("Invalid user".into()))?;
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let limits = get_user_limits(&state.pool, tenant_id).await;
    enforce_card_limit(&state.pool, tenant_id).await?;

    if body.video_provider.is_some() && !limits.allow_videos {
        return Err(AppError::Forbidden("Video requires Pro or Enterprise plan".into()));
    }

    // If layout_blocks are provided, enforce plan limits
    if let Some(ref blocks) = body.layout_blocks {
        enforce_minipage_access(&state.pool, tenant_id).await?;
        enforce_minifunnel_access(&state.pool, tenant_id, blocks).await?;
    }

    let id = Uuid::new_v4();
    let default_blocks: serde_json::Value = body.layout_blocks.unwrap_or(json!([]));

    sqlx::query(
        r#"INSERT INTO kinetic_cards
           (id, user_id, tenant_id, slug, title, tagline, bio, meta_description, logo_url, avatar_url, template_type,
            video_provider, video_id, bg_color, text_color, accent_color,
            button_bg_color, button_text_color,
            instagram_url, facebook_url, twitter_url, youtube_url, linkedin_url, tiktok_url,
            layout_blocks)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25)"#
    )
    .bind(id).bind(user_id).bind(tenant_id)
    .bind(&body.slug).bind(&body.title).bind(&body.tagline).bind(&body.bio).bind(&body.meta_description)
    .bind(&body.logo_url).bind(&body.avatar_url)
    .bind(body.template_type.as_deref().unwrap_or("default"))
    .bind(&body.video_provider).bind(&body.video_id)
    .bind(body.bg_color.as_deref().unwrap_or("#0f172a"))
    .bind(body.text_color.as_deref().unwrap_or("#ffffff"))
    .bind(body.accent_color.as_deref().unwrap_or("#8b5cf6"))
    .bind(body.button_bg_color.as_deref().unwrap_or("#1e293b"))
    .bind(body.button_text_color.as_deref().unwrap_or("#ffffff"))
    .bind(&body.instagram_url).bind(&body.facebook_url).bind(&body.twitter_url)
    .bind(&body.youtube_url).bind(&body.linkedin_url).bind(&body.tiktok_url)
    .bind(&default_blocks)
    .execute(&state.pool).await?;

    let card = sqlx::query_as::<_, KineticCard>("SELECT * FROM kinetic_cards WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok(Json(json!({ "card": card })))
}

pub async fn update_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCardInput>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let limits = get_user_limits(&state.pool, tenant_id).await;

    if body.video_provider.is_some() && !limits.allow_videos {
        return Err(AppError::Forbidden("Video requires Pro or Enterprise plan".into()));
    }

    if let Some(ref blocks) = body.layout_blocks {
        enforce_minipage_access(&state.pool, tenant_id).await?;
        enforce_minifunnel_access(&state.pool, tenant_id, blocks).await?;
    }

    let existing = sqlx::query("SELECT * FROM kinetic_cards WHERE id = $1 AND tenant_id = $2")
        .bind(id).bind(tenant_id)
        .fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Kinetic card not found".into()))?;

    let slug = body.slug.unwrap_or_else(|| existing.get("slug"));
    let title = body.title.unwrap_or_else(|| existing.get("title"));

    sqlx::query(
        r#"UPDATE kinetic_cards SET
           slug=$1,title=$2,tagline=$3,bio=$4,meta_description=$5,logo_url=$6,avatar_url=$7,template_type=$8,
           video_provider=$9,video_id=$10,bg_color=$11,text_color=$12,accent_color=$13,
           button_bg_color=$14,button_text_color=$15,instagram_url=$16,facebook_url=$17,
           twitter_url=$18,youtube_url=$19,linkedin_url=$20,tiktok_url=$21,layout_blocks=$22,
           updated_at=NOW()
           WHERE id=$23 AND tenant_id=$24"#
    )
    .bind(&slug).bind(&title).bind(&body.tagline).bind(&body.bio).bind(&body.meta_description)
    .bind(&body.logo_url).bind(&body.avatar_url)
    .bind(body.template_type.as_deref().unwrap_or_else(|| existing.get("template_type")))
    .bind(&body.video_provider).bind(&body.video_id)
    .bind(body.bg_color.as_deref().unwrap_or_else(|| existing.get("bg_color")))
    .bind(body.text_color.as_deref().unwrap_or_else(|| existing.get("text_color")))
    .bind(body.accent_color.as_deref().unwrap_or_else(|| existing.get("accent_color")))
    .bind(body.button_bg_color.as_deref().unwrap_or_else(|| existing.get("button_bg_color")))
    .bind(body.button_text_color.as_deref().unwrap_or_else(|| existing.get("button_text_color")))
    .bind(&body.instagram_url).bind(&body.facebook_url).bind(&body.twitter_url)
    .bind(&body.youtube_url).bind(&body.linkedin_url).bind(&body.tiktok_url)
    .bind(&body.layout_blocks)
    .bind(id).bind(tenant_id)
    .execute(&state.pool).await?;

    let card = sqlx::query_as::<_, KineticCard>("SELECT * FROM kinetic_cards WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok(Json(json!({ "card": card })))
}

pub async fn delete_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM kinetic_cards WHERE id = $1 AND tenant_id = $2")
        .bind(id).bind(tenant_id).execute(&state.pool).await?;
    Ok(Json(json!({ "status": "deleted" })))
}

// ──────────────────────────────────────────────
// CRUD — Buttons
// ──────────────────────────────────────────────

pub async fn list_buttons(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let buttons = sqlx::query_as::<_, KineticButton>(
        "SELECT * FROM kinetic_buttons WHERE card_id = $1 ORDER BY sort_order ASC"
    ).bind(card_id).fetch_all(&state.pool).await?;
    Ok(Json(json!({ "buttons": buttons })))
}

pub async fn create_button(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateButtonInput>,
) -> AppResult<Json<Value>> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO kinetic_buttons (id, card_id, label, sort_order, action_type, destination_url, target_tag_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)"
    ).bind(id).bind(body.card_id).bind(&body.label)
     .bind(body.sort_order.unwrap_or(0)).bind(&body.action_type)
     .bind(&body.destination_url).bind(&body.target_tag_id)
     .execute(&state.pool).await?;

    let btn = sqlx::query_as::<_, KineticButton>("SELECT * FROM kinetic_buttons WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok(Json(json!({ "button": btn })))
}

pub async fn delete_button(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM kinetic_buttons WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({ "status": "deleted" })))
}

// ──────────────────────────────────────────────
// CRUD — Sources
// ──────────────────────────────────────────────

pub async fn list_sources(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let sources = sqlx::query_as::<_, KineticSource>(
        "SELECT * FROM kinetic_sources WHERE card_id = $1"
    ).bind(card_id).fetch_all(&state.pool).await?;
    Ok(Json(json!({ "sources": sources })))
}

pub async fn create_source(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateSourceInput>,
) -> AppResult<Json<Value>> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO kinetic_sources (id, card_id, source_param, target_tag_id, label) VALUES ($1,$2,$3,$4,$5)"
    ).bind(id).bind(body.card_id).bind(&body.source_param).bind(body.target_tag_id).bind(&body.label)
     .execute(&state.pool).await?;

    let src = sqlx::query_as::<_, KineticSource>("SELECT * FROM kinetic_sources WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok(Json(json!({ "source": src })))
}

pub async fn delete_source(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM kinetic_sources WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({ "status": "deleted" })))
}

// ──────────────────────────────────────────────
// DASHBOARD ANALYTICS
// ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DashboardMetrics {
    pub total_cards: i64,
    pub total_page_views: i64,
    pub total_button_clicks: i64,
    pub total_form_submits: i64,
    pub leads_by_source: Vec<SourceMetric>,
    pub top_tags: Vec<TagMetric>,
    pub views_today: i64,
    pub clicks_today: i64,
    pub views_over_time: Vec<TimeSeriesPoint>,
    pub clicks_over_time: Vec<TimeSeriesPoint>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SourceMetric {
    pub source: String,
    pub source_label: String,
    pub count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TagMetric {
    pub tag_name: String,
    pub count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TimeSeriesPoint {
    pub date: String,
    pub count: i64,
}

pub async fn get_metrics(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<DashboardMetrics>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let total_cards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1")
        .bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);
    let total_page_views: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'page_view'"
    ).bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);
    let total_button_clicks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'button_click'"
    ).bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);
    let total_form_submits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'form_submit'"
    ).bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);

    let views_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id=$1 AND event_type='page_view' AND created_at>=date_trunc('day',NOW())"
    ).bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);
    let clicks_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id=$1 AND event_type='button_click' AND created_at>=date_trunc('day',NOW())"
    ).bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);

    let leads_by_source: Vec<SourceMetric> = sqlx::query_as(
        "SELECT COALESCE(source_param,'direct') as source, COALESCE(source_label,'') as source_label, COUNT(*)::bigint as count \
         FROM lead_events WHERE tenant_id=$1 AND event_type='page_view' \
         GROUP BY source_param, source_label ORDER BY count DESC"
    ).bind(tenant_id).fetch_all(&state.pool).await?;

    let top_tags: Vec<TagMetric> = sqlx::query_as(
        r#"SELECT COALESCE(t.name,'untagged') as tag_name, COUNT(*)::bigint as count
           FROM lead_events le
           LEFT JOIN kinetic_sources ks ON ks.card_id=le.card_id AND ks.source_param=le.source_param
           LEFT JOIN tags t ON t.id=ks.target_tag_id
           WHERE le.tenant_id=$1 GROUP BY t.name ORDER BY count DESC LIMIT 10"#
    ).bind(tenant_id).fetch_all(&state.pool).await?;

    let views_over_time: Vec<TimeSeriesPoint> = sqlx::query_as(
        r#"SELECT to_char(date_trunc('day',created_at),'YYYY-MM-DD') as date, COUNT(*)::bigint as count
           FROM lead_events WHERE tenant_id=$1 AND event_type='page_view' AND created_at>=NOW()-INTERVAL '7 days'
           GROUP BY date_trunc('day',created_at) ORDER BY date"#
    ).bind(tenant_id).fetch_all(&state.pool).await?;

    let clicks_over_time: Vec<TimeSeriesPoint> = sqlx::query_as(
        r#"SELECT to_char(date_trunc('day',created_at),'YYYY-MM-DD') as date, COUNT(*)::bigint as count
           FROM lead_events WHERE tenant_id=$1 AND event_type='button_click' AND created_at>=NOW()-INTERVAL '7 days'
           GROUP BY date_trunc('day',created_at) ORDER BY date"#
    ).bind(tenant_id).fetch_all(&state.pool).await?;

    Ok(Json(DashboardMetrics {
        total_cards, total_page_views, total_button_clicks, total_form_submits,
        leads_by_source, top_tags, views_today, clicks_today,
        views_over_time, clicks_over_time,
    }))
}
