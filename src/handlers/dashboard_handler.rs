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

    // The TENANT ADMIN console (admin.funnelswift.net) paints a different four tiles from the SAME
    // route: Total Leads, Cards Created, Affiliates and Tags, reading `total_cards`,
    // `total_affiliates` and `total_tags`. None of the three was ever emitted here, so three of its
    // four tiles fell back to `||0` and showed a hard zero for EVERY workspace (the fourth,
    // `total_leads`, was the one name this endpoint did return). Same class as the app console's
    // cards_total/plans_total bug fixed in 00a5b2b; this is its admin-console half. Each count
    // mirrors the tenant-scoped LIST the console opens from the tile:
    //   total_cards      == GET /api/v1/kinetic/cards      (WHERE tenant_id = $1)
    //   total_affiliates == GET /api/v1/affiliates         (WHERE tenant_id = $1)
    //   total_tags       == GET /api/v1/tags               (tenant_id = $1 OR is_system = true)
    let total_cards: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kinetic_cards WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?;

    let total_affiliates: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM affiliates WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?;

    let total_tags: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE tenant_id = $1 OR is_system = true")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?;

    // THE WORKSPACE'S OWN TAGS, counted the way the PLAN counts them (2026-11-15, cron `default`).
    // Measured live on a brand-new Capture Free workspace: `/dashboard/stats` answered
    // `total_tags: 9` while `tags` held ZERO rows for that tenant — the nine are the install's own
    // SYSTEM labels, which every workspace can use. The admin console paints `total_tags` under the
    // bare label "Tags" beside Total Leads / Cards Created / Affiliates, and all three of those are
    // strictly tenant-scoped, so the tile told an operator the workspace had nine tags of its own.
    // The same screen's plan line reads `GET /api/v1/me/usage` -> `tags`, which counts
    // `is_system = false` (the figure the tag GATE compares against `max_tags`), so the two numbers
    // on ONE screen contradicted each other: "Tags 9" beside "Tags 0 / 2".
    // `total_tags` is left exactly as it was — it mirrors the list `GET /api/v1/tags` opens, which
    // includes the system rows, and that list is unchanged. This adds the honest count beside it.
    let tags_own: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE tenant_id = $1 AND is_system = false")
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
        "total_cards": total_cards,
        "total_affiliates": total_affiliates,
        "total_tags": total_tags,
        "tags_own": tags_own,
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
