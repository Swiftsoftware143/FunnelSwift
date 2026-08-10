use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// GET /api/v1/admin/plans/:id/templates — get allowed template IDs for a plan
pub async fn get_plan_templates(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let plan = sqlx::query("SELECT name, slug, allowed_template_ids FROM plans WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;
    let allowed: Option<Vec<String>> = plan.try_get("allowed_template_ids").unwrap_or(None);
    let name: String = plan.try_get("name").unwrap_or_default();
    let slug: String = plan.try_get("slug").unwrap_or_default();
    Ok(Json(json!({
        "plan_id": id.to_string(),
        "plan_name": name,
        "plan_slug": slug,
        "allowed_template_ids": allowed,
        "rule": if allowed.is_none() { "all" } else if allowed.as_ref().map(|a| a.is_empty()).unwrap_or(false) { "none" } else { "specific" }
    })))
}

/// PUT /api/v1/admin/plans/:id/templates — set which templates a plan allows
/// { "allowed_template_ids": ["biz_executive", "page_saas"] } → only those
/// { "allowed_template_ids": [] } → zero templates (locked down)
/// { "allowed_template_ids": null } → all templates (unlocked)
pub async fn update_plan_templates(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> AppResult<Json<Value>> {
    let template_ids: Option<Vec<String>> = match req.get("allowed_template_ids") {
        Some(v) if v.is_null() => None, // null = all
        Some(v) => v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        }),
        None => {
            return Err(AppError::BadRequest(
                "allowed_template_ids field required".into(),
            ))
        }
    };

    sqlx::query("UPDATE plans SET allowed_template_ids = $1 WHERE id = $2")
        .bind(&template_ids)
        .bind(id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({
        "message": if template_ids.is_none() { "All templates enabled" } else if template_ids.as_ref().map(|a| a.is_empty()).unwrap_or(false) { "All templates disabled" } else { "Template list updated" },
        "plan_id": id.to_string(),
        "allowed_template_ids": template_ids
    })))
}
