use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, Json};

/// `/api/v1/admin/site` is on the platform-admin surface (kanban t_9cf378bc). This pair is an
/// inert stub with no caller anywhere in the app, its mirror or the docs, but it is registered
/// on the admin router, so it carries the same role check as every other handler there rather
/// than relying on the router choke point alone. The body is unchanged.
pub async fn get_site(
    auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !crate::auth::global_auth::is_platform_admin(&auth.role) {
        return Err(AppError::Forbidden("Platform admin role required".into()));
    }
    Ok(Json(serde_json::json!({"status": "ok"})))
}

pub async fn update_site(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !crate::auth::global_auth::is_platform_admin(&auth.role) {
        return Err(AppError::Forbidden("Platform admin role required".into()));
    }
    Ok(Json(serde_json::json!({"status": "ok"})))
}
