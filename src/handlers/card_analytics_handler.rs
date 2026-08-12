// ── Card Analytics + UTM Handler ──
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize, Debug)]
pub struct TrackQuery {
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
    pub utm_term: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct TrackCardEventRequest {
    pub event_type: String,
    pub user_agent: Option<String>,
    pub referrer_url: Option<String>,
    pub device_type: Option<String>,
    pub screen_size: Option<String>,
    pub click_label: Option<String>,
    pub click_url: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
    pub utm_term: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct UpdateCardUtmRequest {
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
    pub utm_term: Option<String>,
}

/// Classify a referrer URL into UTM source/medium when no UTMs are present.
/// Returns None if the referrer doesn't match any known pattern.
fn classify_referrer(referrer: &str, device_type: &Option<String>) -> Option<(String, String)> {
    let is_mobile = device_type.as_deref() == Some("mobile");
    let ref_lower = referrer.to_lowercase();

    if ref_lower.is_empty() {
        // No referrer at all — mobile = likely shared via AirDrop/text/QR
        if is_mobile {
            return Some(("direct".into(), "mobile-share".into()));
        }
        // Desktop with no referrer = typed URL, bookmark, or email client
        return Some(("direct".into(), "none".into()));
    }

    // Search engines
    if ref_lower.contains("google.com/search") || ref_lower.contains("google.co") {
        return Some(("google".into(), "organic".into()));
    }
    if ref_lower.contains("bing.com/search") {
        return Some(("bing".into(), "organic".into()));
    }
    if ref_lower.contains("duckduckgo.com") {
        return Some(("duckduckgo".into(), "organic".into()));
    }
    if ref_lower.contains("yahoo.com/search") {
        return Some(("yahoo".into(), "organic".into()));
    }
    if ref_lower.contains("yandex.") {
        return Some(("yandex".into(), "organic".into()));
    }

    // Social media
    if ref_lower.contains("facebook.com") || ref_lower.contains("fb.com") {
        return Some(("facebook".into(), "social".into()));
    }
    if ref_lower.contains("instagram.com") {
        return Some(("instagram".into(), "social".into()));
    }
    if ref_lower.contains("linkedin.com") {
        return Some(("linkedin".into(), "social".into()));
    }
    if ref_lower.contains("twitter.com")
        || ref_lower.contains("t.co")
        || ref_lower.contains("x.com")
    {
        return Some(("twitter".into(), "social".into()));
    }
    if ref_lower.contains("tiktok.com") {
        return Some(("tiktok".into(), "social".into()));
    }
    if ref_lower.contains("reddit.com") {
        return Some(("reddit".into(), "social".into()));
    }
    if ref_lower.contains("pinterest.com") || ref_lower.contains("pin.it") {
        return Some(("pinterest".into(), "social".into()));
    }
    if ref_lower.contains("youtube.com") {
        return Some(("youtube".into(), "social".into()));
    }
    if ref_lower.contains("snapchat.com") {
        return Some(("snapchat".into(), "social".into()));
    }
    if ref_lower.contains("whatsapp.com") {
        return Some(("whatsapp".into(), "social".into()));
    }
    if ref_lower.contains("telegram.org") || ref_lower.contains("t.me") {
        return Some(("telegram".into(), "social".into()));
    }
    if ref_lower.contains("discord.com") || ref_lower.contains("discord.gg") {
        return Some(("discord".into(), "social".into()));
    }
    if ref_lower.contains("slack.com") {
        return Some(("slack".into(), "social".into()));
    }

    // Email clients (webmail referrers)
    if ref_lower.contains("mail.google.com")
        || ref_lower.contains("mail.yahoo.com")
        || ref_lower.contains("outlook.live.com")
        || ref_lower.contains("outlook.office.com")
        || ref_lower.contains("mail.proton")
    {
        return Some(("email".into(), "email".into()));
    }

    // Known sites — use domain as source
    if let Some(domain) = extract_domain_from_url(referrer) {
        return Some((domain, "referral".into()));
    }

    None
}

/// Extract a clean domain from a URL string for referral attribution.
fn extract_domain_from_url(url: &str) -> Option<String> {
    let without_proto = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let domain = without_proto.split('/').next().unwrap_or(without_proto);
    if domain.is_empty() || domain.contains(' ') {
        return None;
    }
    Some(domain.to_string())
}

pub async fn track_card_event(
    Path(card_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(payload): Json<TrackCardEventRequest>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    // Find tenant from card
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT tenant_id FROM kinetic_cards WHERE id = $1")
        .bind(card_id)
        .fetch_optional(&state.pool)
        .await
        .unwrap_or_default();

    let tenant_id = match row {
        Some((tid,)) => tid,
        None => {
            return (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({"status":"ignored","reason":"card_not_found"})),
            )
        }
    };

    let event_id = Uuid::new_v4();
    let event_type = payload.event_type.clone();

    // Use UTM from payload (JS tracker parses URL params)
    let mut utm_source = payload.utm_source.clone();
    let mut utm_medium = payload.utm_medium.clone();
    let utm_campaign = payload.utm_campaign.clone();
    let utm_content = payload.utm_content.clone();
    let utm_term = payload.utm_term.clone();

    // Referrer-based attribution fallback — when no UTMs are present,
    // classify traffic by referrer URL so mobile shares and direct links
    // still show up as attributed traffic in analytics.
    if utm_source.is_none() {
        let ref_url = payload
            .referrer_url
            .clone()
            .unwrap_or_default()
            .to_lowercase();
        if let Some((src, med)) = classify_referrer(&ref_url, &payload.device_type) {
            utm_source = Some(src);
            utm_medium = Some(med);
        }
    }

    // Parse browser/OS from user agent
    let ua = payload.user_agent.clone().unwrap_or_default();
    let (browser_family, os_family) = parse_user_agent(&ua);

    // Insert raw event
    sqlx::query(
        "INSERT INTO kinetic_card_events (id, card_id, tenant_id, event_type, utm_source, utm_medium, utm_campaign, utm_content, utm_term, user_agent, browser_family, os_family, referrer_url, device_type, screen_size, click_label, click_url, ip_address, session_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"
    )
    .bind(event_id)
    .bind(card_id)
    .bind(tenant_id)
    .bind(&event_type)
    .bind(&utm_source)
    .bind(&utm_medium)
    .bind(&utm_campaign)
    .bind(&utm_content)
    .bind(&utm_term)
    .bind(&ua)
    .bind(&browser_family)
    .bind(&os_family)
    .bind(&payload.referrer_url)
    .bind(&payload.device_type)
    .bind(&payload.screen_size)
    .bind(&payload.click_label)
    .bind(&payload.click_url)
    .bind(Option::<String>::None) // ip_address — skip for privacy
    .bind(Option::<String>::None) // session_id
    .execute(&state.pool)
    .await.unwrap_or_default();

    // Upsert daily stats
    let today = chrono::Utc::now().date_naive();
    let is_view = event_type == "view";
    let is_click =
        event_type == "click" || event_type == "button_click" || event_type == "link_click";
    let is_share = event_type == "share";

    sqlx::query(
        "INSERT INTO kinetic_card_daily_stats (id, card_id, tenant_id, stat_date, views, clicks, shares) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6) ON CONFLICT (card_id, stat_date) DO UPDATE SET views = kinetic_card_daily_stats.views + $4, clicks = kinetic_card_daily_stats.clicks + $5, shares = kinetic_card_daily_stats.shares + $6"
    )
    .bind(card_id)
    .bind(tenant_id)
    .bind(today)
    .bind(if is_view { 1 } else { 0 })
    .bind(if is_click { 1 } else { 0 })
    .bind(if is_share { 1 } else { 0 })
    .execute(&state.pool)
    .await.unwrap_or_default();

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({"status":"ok","event_id":event_id.to_string()})),
    )
}

// ── Get card-specific analytics ──
#[derive(Serialize, Default)]
#[serde(default)]
pub struct CardAnalyticsResponse {
    pub card_id: String,
    pub card_title: String,
    pub total_views: i64,
    pub total_clicks: i64,
    pub total_shares: i64,
    pub unique_visitors: i64,
    pub today_views: i64,
    pub this_week_views: i64,
    pub utm_sources: Vec<UtmSourceRow>,
    pub top_locations: Vec<LocationRow>,
    pub daily_stats: Vec<DailyStatsRow>,
    pub events_by_type: Vec<EventTypeRow>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct UtmSourceRow {
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub count: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct LocationRow {
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub count: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DailyStatsRow {
    pub date: Option<chrono::NaiveDate>,
    pub views: Option<i64>,
    pub clicks: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct EventTypeRow {
    pub event_type: Option<String>,
    pub count: Option<i64>,
}

pub async fn get_card_analytics(
    Path(card_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<CardAnalyticsResponse>> {
    // Verify card exists
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kinetic_cards WHERE id=$1)")
        .bind(card_id)
        .fetch_one(&state.pool)
        .await?;

    if !exists {
        return Err(AppError::NotFound("Card not found".into()));
    }

    let total_views: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_card_events WHERE card_id=$1 AND event_type='view'",
    )
    .bind(card_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let total_clicks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_card_events WHERE card_id=$1 AND event_type IN ('click','button_click','link_click')"
    ).bind(card_id).fetch_one(&state.pool).await.unwrap_or(0);

    let total_shares: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_card_events WHERE card_id=$1 AND event_type='share'",
    )
    .bind(card_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let unique_visitors: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT session_id) FROM kinetic_card_events WHERE card_id=$1 AND session_id IS NOT NULL"
    ).bind(card_id).fetch_one(&state.pool).await.unwrap_or(0);

    let today_views: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_card_events WHERE card_id=$1 AND event_type='view' AND created_at::date = CURRENT_DATE"
    ).bind(card_id).fetch_one(&state.pool).await.unwrap_or(0);

    let this_week_views: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_card_events WHERE card_id=$1 AND event_type='view' AND created_at >= date_trunc('week', CURRENT_DATE)"
    ).bind(card_id).fetch_one(&state.pool).await.unwrap_or(0);

    let utm_sources: Vec<UtmSourceRow> = sqlx::query_as(
        "SELECT utm_source, utm_medium, utm_campaign, COUNT(*) as count FROM kinetic_card_events WHERE card_id=$1 AND utm_source IS NOT NULL GROUP BY utm_source, utm_medium, utm_campaign ORDER BY count DESC LIMIT 10"
    ).bind(card_id).fetch_all(&state.pool).await.unwrap_or_default();

    let top_locations: Vec<LocationRow> = sqlx::query_as(
        "SELECT country, region, city, view_count as count FROM kinetic_card_locations WHERE card_id=$1 ORDER BY view_count DESC LIMIT 20"
    ).bind(card_id).fetch_all(&state.pool).await.unwrap_or_default();

    let daily_stats: Vec<DailyStatsRow> = sqlx::query_as(
        "SELECT stat_date as date, views, clicks FROM kinetic_card_daily_stats WHERE card_id=$1 ORDER BY stat_date DESC LIMIT 30"
    ).bind(card_id).fetch_all(&state.pool).await.unwrap_or_default();

    let events_by_type: Vec<EventTypeRow> = sqlx::query_as(
        "SELECT event_type, COUNT(*) as count FROM kinetic_card_events WHERE card_id=$1 GROUP BY event_type ORDER BY count DESC"
    ).bind(card_id).fetch_all(&state.pool).await.unwrap_or_default();

    Ok(Json(CardAnalyticsResponse {
        card_id: card_id.to_string(),
        card_title: String::new(), // Will be populated from the card data if loaded
        total_views,
        total_clicks,
        total_shares,
        unique_visitors,
        today_views,
        this_week_views,
        utm_sources,
        top_locations,
        daily_stats,
        events_by_type,
    }))
}

// ── Tenant analytics overview ──
#[derive(Serialize, Default, sqlx::FromRow)]
#[serde(default)]
pub struct TenantAnalyticsSummary {
    pub total_cards: i64,
    pub active_cards: i64,
    pub total_views: i64,
    pub total_clicks: i64,
    pub unique_visitors: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub top_cards: Vec<CardPerformanceRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub top_countries: Vec<LocationRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub daily_trend: Vec<DailyStatsRow>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct CardPerformanceRow {
    pub card_id: Option<Uuid>,
    pub card_title: Option<String>,
    pub card_type: Option<String>,
    pub views: Option<i64>,
    pub clicks: Option<i64>,
    pub shares: Option<i64>,
}

pub async fn get_tenant_analytics(
    State(state): State<AppState>,
    auth: crate::auth::middleware::AuthUser,
) -> AppResult<Json<TenantAnalyticsSummary>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let total_cards: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id=$1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);

    let active_cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id=$1 AND is_active=true",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let total_views: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(views), 0) FROM kinetic_card_daily_stats WHERE tenant_id=$1",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let total_clicks: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(clicks), 0) FROM kinetic_card_daily_stats WHERE tenant_id=$1",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let unique_visitors: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT session_id) FROM kinetic_card_events WHERE tenant_id=$1 AND session_id IS NOT NULL"
    ).bind(tenant_id).fetch_one(&state.pool).await.unwrap_or(0);

    let top_cards: Vec<CardPerformanceRow> = sqlx::query_as(
        "SELECT k.id as card_id, k.title as card_title, k.template_type as card_type,
                COALESCE(d.views, 0) as views, COALESCE(d.clicks, 0) as clicks, COALESCE(d.shares, 0) as shares
         FROM kinetic_cards k
         LEFT JOIN (SELECT card_id, SUM(views) as views, SUM(clicks) as clicks, SUM(shares) as shares
                    FROM kinetic_card_daily_stats GROUP BY card_id) d ON d.card_id = k.id
         WHERE k.tenant_id = $1
         ORDER BY views DESC LIMIT 10"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();

    let top_countries: Vec<LocationRow> = sqlx::query_as(
        "SELECT country, region, city, SUM(view_count) as count FROM kinetic_card_locations WHERE tenant_id=$1 GROUP BY country, region, city ORDER BY count DESC LIMIT 20"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();

    let daily_trend: Vec<DailyStatsRow> = sqlx::query_as(
        "SELECT stat_date as date, SUM(views) as views, SUM(clicks) as clicks FROM kinetic_card_daily_stats WHERE tenant_id=$1 GROUP BY stat_date ORDER BY stat_date DESC LIMIT 30"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();

    Ok(Json(TenantAnalyticsSummary {
        total_cards,
        active_cards,
        total_views,
        total_clicks,
        unique_visitors,
        top_cards,
        top_countries,
        daily_trend,
    }))
}

// ── UTM management ──
pub async fn update_card_utm(
    Path(card_id): Path<Uuid>,
    State(state): State<AppState>,
    auth: crate::auth::middleware::AuthUser,
    Json(payload): Json<UpdateCardUtmRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query(
        "UPDATE kinetic_cards SET utm_source=$3, utm_medium=$4, utm_campaign=$5, utm_content=$6, utm_term=$7 WHERE id=$1 AND tenant_id=$2"
    )
    .bind(card_id)
    .bind(tenant_id)
    .bind(&payload.utm_source)
    .bind(&payload.utm_medium)
    .bind(&payload.utm_campaign)
    .bind(&payload.utm_content)
    .bind(&payload.utm_term)
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({"status":"ok"})))
}

pub async fn get_card_tracking_url(
    Path(card_id): Path<Uuid>,
    State(state): State<AppState>,
    auth: crate::auth::middleware::AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let row: Option<(String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT slug, utm_source, utm_medium, utm_campaign, utm_content, utm_term FROM kinetic_cards WHERE id=$1 AND tenant_id=$2"
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let (slug, utm_source, utm_medium, utm_campaign, utm_content, utm_term) =
        row.ok_or_else(|| AppError::NotFound("Card not found".into()))?;

    // Resolve correct domain: user slug on kntcrd.com, custom domain, or funnelswift redirect
    let tenant_slug: Option<String> = sqlx::query_scalar("SELECT slug FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await
        .unwrap_or(None);

    let base_url = if let Some(ref ts) = tenant_slug {
        format!("https://{}.kntcrd.com", ts)
    } else {
        "https://funnelswift.net".to_string()
    };
    let mut url = format!("{}/k/{}", base_url, slug);
    let mut params = vec![];
    if let Some(ref s) = utm_source {
        params.push(format!("utm_source={}", s));
    }
    if let Some(ref s) = utm_medium {
        params.push(format!("utm_medium={}", s));
    }
    if let Some(ref s) = utm_campaign {
        params.push(format!("utm_campaign={}", s));
    }
    if let Some(ref s) = utm_content {
        params.push(format!("utm_content={}", s));
    }
    if let Some(ref s) = utm_term {
        params.push(format!("utm_term={}", s));
    }
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }

    Ok(Json(serde_json::json!({"tracking_url": url, "slug": slug})))
}

// ── Helpers ──
fn parse_user_agent(ua: &str) -> (Option<String>, Option<String>) {
    let ua_lower = ua.to_lowercase();
    let browser = if ua_lower.contains("firefox") {
        Some("Firefox".into())
    } else if ua_lower.contains("edg") {
        Some("Edge".into())
    } else if ua_lower.contains("chrome") && !ua_lower.contains("edg") {
        Some("Chrome".into())
    } else if ua_lower.contains("safari") && !ua_lower.contains("chrome") {
        Some("Safari".into())
    } else if ua_lower.contains("opera") {
        Some("Opera".into())
    } else {
        None
    };

    let os = if ua_lower.contains("windows") {
        Some("Windows".into())
    } else if ua_lower.contains("mac os") || ua_lower.contains("macos") {
        Some("macOS".into())
    } else if ua_lower.contains("linux") && !ua_lower.contains("android") {
        Some("Linux".into())
    } else if ua_lower.contains("android") {
        Some("Android".into())
    } else if ua_lower.contains("iphone") || ua_lower.contains("ipad") || ua_lower.contains("ios") {
        Some("iOS".into())
    } else {
        None
    };

    (browser, os)
}
