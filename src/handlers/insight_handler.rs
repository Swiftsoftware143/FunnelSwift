use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// `GET /api/v1/insights?days=30` — the workspace's own capture performance.
///
/// WHY THIS SHAPE (cron 2026-11-01). The route answered `{"total_leads": N}` and NO console called
/// it (grep -c insights over the served app console = 0), while the user guide tells a customer to
/// "use different slugs for different traffic sources … so you can track which source converts best
/// via Insights". So the guide promised a measurement the product could not produce: one number is
/// not a source comparison. This answers the guide's own sentence — every traffic source, how many
/// leads it brought in the window, how many of those became customers, and the conversion rate.
///
/// SCOPE: `tenant_id` comes from the credential, never from the query string — the same rule the
/// leaked-counts fix applied. Read-only: one SELECT per block, no writes at all.
///
/// CONVERSION VOCABULARY: `leads.status` is the app's own five-value lifecycle
/// (new, contacted, qualified, converted, lost — see `lead_handler`'s note). A converted lead is
/// `status = 'converted'` and nothing else; `qualified` is interest, not a customer.
#[derive(Debug, Deserialize)]
pub struct InsightsQuery {
    pub days: Option<i64>,
}

pub async fn get_dashboard_insights(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<InsightsQuery>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // The window is the caller's choice, clamped so a hostile or fat-fingered value cannot ask the
    // database for a decade of history (or a zero-width window that reports nothing at all).
    let days: i32 = q.days.unwrap_or(30).clamp(1, 365) as i32;

    // Every lead this workspace has ever captured — the number the dashboard already shows.
    let lead_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM leads WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or((0,));

    // ── per-source rows: the guide's promise ─────────────────────────────────────────────────────
    // A blank `source` is a lead with no attribution (a manual entry, an import, a card with no
    // slug). It is reported as `(direct)` rather than dropped, so the rows always add up to the
    // window total instead of quietly losing the untagged leads.
    let source_rows = sqlx::query(
        r#"SELECT COALESCE(NULLIF(btrim(source), ''), '(direct)') AS source,
                  COUNT(*)::bigint AS leads,
                  COUNT(*) FILTER (
                      WHERE lower(btrim(COALESCE(status, ''))) = 'converted'
                  )::bigint AS converted,
                  MAX(created_at)::text AS last_lead_at
             FROM leads
            WHERE tenant_id = $1
              AND created_at >= NOW() - make_interval(days => $2)
            GROUP BY 1
            ORDER BY leads DESC, source ASC
            LIMIT 50"#,
    )
    .bind(tenant_id)
    .bind(days)
    .fetch_all(&state.pool)
    .await?;

    let mut sources = Vec::with_capacity(source_rows.len());
    let mut window_leads: i64 = 0;
    let mut converted: i64 = 0;
    for row in &source_rows {
        let source: String = row.get("source");
        let leads: i64 = row.get("leads");
        let conv: i64 = row.get("converted");
        let last_lead_at: Option<String> = row.try_get("last_lead_at").ok().flatten();
        window_leads += leads;
        converted += conv;
        sources.push(json!({
            "source": source,
            "leads": leads,
            "converted": conv,
            // One decimal is what a person reads; the raw fraction is not useful here and a
            // division by zero on a source with no leads must not become an error.
            "conversion_rate": if leads > 0 {
                ((conv as f64) * 1000.0 / (leads as f64)).round() / 10.0
            } else {
                0.0
            },
            "last_lead_at": last_lead_at,
        }));
    }

    // ── daily trend: the same window, one row per day that HAS a lead ───────────────────────────
    let day_rows = sqlx::query(
        r#"SELECT to_char(date_trunc('day', created_at), 'YYYY-MM-DD') AS day,
                  COUNT(*)::bigint AS leads
             FROM leads
            WHERE tenant_id = $1
              AND created_at >= NOW() - make_interval(days => $2)
            GROUP BY 1
            ORDER BY 1"#,
    )
    .bind(tenant_id)
    .bind(days)
    .fetch_all(&state.pool)
    .await?;
    let by_day: Vec<Value> = day_rows
        .iter()
        .map(|r| {
            let day: String = r.get("day");
            let leads: i64 = r.get("leads");
            json!({ "day": day, "leads": leads })
        })
        .collect();

    // ── status mix: where the window's leads actually sit in the lifecycle ──────────────────────
    let status_rows = sqlx::query(
        r#"SELECT COALESCE(NULLIF(btrim(status), ''), '(unset)') AS status,
                  COUNT(*)::bigint AS leads
             FROM leads
            WHERE tenant_id = $1
              AND created_at >= NOW() - make_interval(days => $2)
            GROUP BY 1
            ORDER BY leads DESC, status ASC"#,
    )
    .bind(tenant_id)
    .bind(days)
    .fetch_all(&state.pool)
    .await?;
    let statuses: Vec<Value> = status_rows
        .iter()
        .map(|r| {
            let status: String = r.get("status");
            let leads: i64 = r.get("leads");
            json!({ "status": status, "leads": leads })
        })
        .collect();

    Ok(Json(json!({
        "total_leads": lead_count.0,
        "window_days": days,
        "window_leads": window_leads,
        "converted": converted,
        "conversion_rate": if window_leads > 0 {
            ((converted as f64) * 1000.0 / (window_leads as f64)).round() / 10.0
        } else {
            0.0
        },
        "sources": sources,
        "by_day": by_day,
        "statuses": statuses,
    })))
}
