use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

pub async fn list_tenants(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    // NOTE: the tenants table has no email/status/is_visible/plan_id columns, so the old
    // query referenced four columns that do not exist and then swallowed the resulting
    // error with `.unwrap_or_default()` — every caller silently received `[]` (112 rows in
    // the DB, empty list in the UI). Plan comes from the newest active subscription row.
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            chrono::NaiveDateTime,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"SELECT t.id, t.name, t.slug, t.created_at, p.name AS plan_name, p.slug AS plan_slug
           FROM tenants t
           LEFT JOIN LATERAL (
               SELECT plan_id FROM tenant_plan_subscriptions s
               WHERE s.tenant_id = t.id AND s.status = 'active'
               ORDER BY s.start_date DESC NULLS LAST LIMIT 1
           ) sub ON TRUE
           LEFT JOIN plans p ON p.id = sub.plan_id
           ORDER BY t.created_at DESC"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let tenants: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "name": r.1, "slug": r.2, "created_at": r.3,
                "plan_name": r.4, "plan_slug": r.5, "plan": r.4, "status": "active"
            })
        })
        .collect();
    Ok(Json(json!(tenants)))
}

pub async fn get_tenant(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!({"id": id})))
}

pub async fn create_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let id = Uuid::new_v4();
    let name = req["name"].as_str().unwrap_or("New Tenant");
    let email = req["email"].as_str().unwrap_or("");
    sqlx::query("INSERT INTO tenants (id, name, email) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(email)
        .execute(&state.pool)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Tenant created"})),
    ))
}

pub async fn update_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    sqlx::query(
        "UPDATE tenants SET name=COALESCE($2, name), email=COALESCE($3, email) WHERE id=$1",
    )
    .bind(id)
    .bind(req["name"].as_str())
    .bind(req["email"].as_str())
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"message": "Tenant updated"})))
}

pub async fn delete_tenant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    sqlx::query("DELETE FROM tenants WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Tenant deleted"})))
}

pub async fn assign_plan(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin only".into()));
    }
    let plan_id = req["plan_id"].as_str().unwrap_or("");
    sqlx::query("UPDATE tenants SET plan_id=$2 WHERE id=$1")
        .bind(id)
        .bind(plan_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Plan assigned"})))
}
