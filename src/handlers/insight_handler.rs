use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use uuid::Uuid;

pub async fn get_dashboard_insights(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // Scope lead metrics to the caller's tenant (previously leaked platform-wide counts).
    let lead_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM leads WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or((0,));
    Ok(Json(json!({"total_leads": lead_count.0})))
}
