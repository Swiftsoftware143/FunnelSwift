use axum::{extract::State, Json};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::activity::ActivityLog;
use crate::models::dashboard::*;
use crate::state::AppState;

pub async fn get_dashboard_stats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let total_leads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leads WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;

    let leads_by_stage: Vec<StageCount> = sqlx::query_as(
        "SELECT COALESCE(stage, 'Uncategorized') as stage, COUNT(*)::bigint as count FROM leads WHERE tenant_id = $1 GROUP BY stage ORDER BY count DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let leads_by_source: Vec<SourceCount> = sqlx::query_as(
        "SELECT COALESCE(source, 'Unknown') as source, COUNT(*)::bigint as count FROM leads WHERE tenant_id = $1 GROUP BY source ORDER BY count DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let leads_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leads WHERE tenant_id = $1 AND created_at::date = CURRENT_DATE",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;

    let leads_this_week: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leads WHERE tenant_id = $1 AND created_at >= date_trunc('week', CURRENT_DATE)",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;

    let leads_this_month: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leads WHERE tenant_id = $1 AND created_at >= date_trunc('month', CURRENT_DATE)",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;

    let conversion_rate: f64 = if total_leads > 0 {
        let won: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM leads WHERE tenant_id = $1 AND stage = 'Closed Won'",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        (won as f64 / total_leads as f64) * 100.0
    } else {
        0.0
    };

    // The console's dashboard paints four tiles: Total Leads, Conv. Rate, Cards and Plans. The last
    // two read `cards_total` / `plans_total`, and this endpoint never returned either — so both fell
    // back to the SPA's `||0` and showed a hard zero for EVERY workspace whatever it owned (the two
    // names were born in the frontend's baseline commit and never had a producer). Both are
    // tenant-scoped counts of rows the CALLER owns, never catalog/global totals: `cards_total` is
    // what the "My Cards" screen lists, `plans_total` is how many subscriptions this workspace holds
    // (the Upgrade screen's "Current" rows).
    let cards_total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?;

    let plans_total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenant_plan_subscriptions WHERE tenant_id = $1 AND status = 'active'",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;

    let stats = serde_json::json!({
        "total_leads": total_leads,
        "leads_by_stage": leads_by_stage,
        "leads_by_source": leads_by_source,
        "conversion_rate": conversion_rate,
        "leads_today": leads_today,
        "leads_this_week": leads_this_week,
        "leads_this_month": leads_this_month,
        "cards_total": cards_total,
        "plans_total": plans_total,
    });

    Ok(Json(stats))
}

pub async fn get_activity_log(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<ActivityLog>>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let activities = sqlx::query_as::<_, ActivityLog>(
        "SELECT * FROM activity_log WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(activities))
}
