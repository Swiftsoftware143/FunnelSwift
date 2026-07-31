use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;

pub async fn get_dashboard_insights(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let lead_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM leads").fetch_one(&state.pool).await.unwrap_or((0,));
    let tenant_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tenants").fetch_one(&state.pool).await.unwrap_or((0,));
    Ok(Json(json!({"total_leads": lead_count.0, "total_tenants": tenant_count.0})))
}
