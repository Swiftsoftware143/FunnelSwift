use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct InsightQuery {
    pub days: Option<i32>,
    pub card_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct DashboardInsights {
    pub total_views: i64,
    pub total_clicks: i64,
    pub total_submits: i64,
    pub conversion_rate: f64,
    pub views_today: i64,
    pub leads_today: i64,
    pub card_count: i64,
    pub top_cards: Vec<CardInsight>,
    pub daily_series: Vec<DailyPoint>,
    pub source_breakdown: Vec<SourceBreakdown>,
}

#[derive(Serialize)]
pub struct CardInsight {
    pub card_id: Uuid,
    pub title: String,
    pub views: i64,
    pub clicks: i64,
    pub submits: i64,
}

#[derive(Serialize)]
pub struct DailyPoint {
    pub date: String,
    pub views: i64,
    pub submits: i64,
}

#[derive(Serialize)]
pub struct SourceBreakdown {
    pub source: String,
    pub count: i64,
}

pub async fn get_dashboard_insights(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<InsightQuery>,
) -> AppResult<Json<DashboardInsights>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let days = q.days.unwrap_or(30).max(1).min(365);
    let user_id: Uuid = auth.user_id.parse().map_err(|_| AppError::BadRequest("Invalid user".into()))?;

    // Total counts
    let total_views: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'page_view' AND created_at >= NOW() - ($2 || ' days')::INTERVAL"
    )
    .bind(tenant_id).bind(&days.to_string())
    .fetch_one(&state.pool).await.unwrap_or(0);

    let total_clicks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'button_click' AND created_at >= NOW() - ($2 || ' days')::INTERVAL"
    )
    .bind(tenant_id).bind(&days.to_string())
    .fetch_one(&state.pool).await.unwrap_or(0);

    let total_submits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'form_submit' AND created_at >= NOW() - ($2 || ' days')::INTERVAL"
    )
    .bind(tenant_id).bind(&days.to_string())
    .fetch_one(&state.pool).await.unwrap_or(0);

    // Today's counts
    let views_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'page_view' AND created_at::date = CURRENT_DATE"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool).await.unwrap_or(0);

    let leads_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leads WHERE tenant_id = $1 AND created_at::date = CURRENT_DATE"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool).await.unwrap_or(0);

    // Card count
    let card_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_one(&state.pool).await.unwrap_or(0);

    // Conversion rate
    let conversion_rate = if total_views > 0 {
        (total_submits as f64 / total_views as f64) * 100.0
    } else {
        0.0
    };

    // Top cards
    let top_cards: Vec<CardInsight> = sqlx::query_as::<_, (Uuid, String, i64, i64, i64)>(
        "SELECT kc.id, kc.title,
            COALESCE((SELECT COUNT(*) FROM lead_events le WHERE le.card_id = kc.id AND le.event_type = 'page_view' AND le.created_at >= NOW() - ($2 || ' days')::INTERVAL), 0) as views,
            COALESCE((SELECT COUNT(*) FROM lead_events le WHERE le.card_id = kc.id AND le.event_type = 'button_click' AND le.created_at >= NOW() - ($2 || ' days')::INTERVAL), 0) as clicks,
            COALESCE((SELECT COUNT(*) FROM lead_events le WHERE le.card_id = kc.id AND le.event_type = 'form_submit' AND le.created_at >= NOW() - ($2 || ' days')::INTERVAL), 0) as submits
         FROM kinetic_cards kc WHERE kc.tenant_id = $1
         ORDER BY views DESC LIMIT 10"
    )
    .bind(tenant_id).bind(&days.to_string())
    .fetch_all(&state.pool).await.unwrap_or_default()
    .into_iter().map(|(id, title, views, clicks, submits)| CardInsight { card_id: id, title, views, clicks, submits })
    .collect();

    // Daily series (last 14 days)
    let daily_series: Vec<DailyPoint> = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT d::date::text as date,
            COALESCE((SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'page_view' AND created_at::date = d), 0) as views,
            COALESCE((SELECT COUNT(*) FROM lead_events WHERE tenant_id = $1 AND event_type = 'form_submit' AND created_at::date = d), 0) as submits
         FROM generate_series(CURRENT_DATE - 13, CURRENT_DATE, '1 day'::interval) d
         ORDER BY d"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool).await.unwrap_or_default()
    .into_iter().map(|(date, views, submits)| DailyPoint { date, views, submits })
    .collect();

    // Source breakdown
    let source_breakdown: Vec<SourceBreakdown> = sqlx::query_as::<_, (String, i64)>(
        "SELECT COALESCE(source_label, source_param, 'direct') as source, COUNT(*) as count
         FROM lead_events WHERE tenant_id = $1 AND event_type = 'page_view' AND created_at >= NOW() - ($2 || ' days')::INTERVAL
         GROUP BY source ORDER BY count DESC LIMIT 10"
    )
    .bind(tenant_id).bind(&days.to_string())
    .fetch_all(&state.pool).await.unwrap_or_default()
    .into_iter().map(|(source, count)| SourceBreakdown { source, count })
    .collect();

    Ok(Json(DashboardInsights {
        total_views, total_clicks, total_submits, conversion_rate,
        views_today, leads_today, card_count,
        top_cards, daily_series, source_breakdown,
    }))
}
