use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// site_settings.value is TEXT, not jsonb. Handlers used to decode it straight into
/// `serde_json::Value`, which fails at the sqlx layer — that made
/// GET /api/v1/admin/sites/:slug answer 500 and made every list/read (admin sites, SEO
/// injection, robots crawl-delay) resolve to an empty default. Reads now go through the
/// text and parse leniently; writes store the JSON text so the round-trip is stable.
pub fn value_from_text(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn text_from_value(v: &Value) -> String {
    match v {
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub async fn list_site_settings(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String)> =
        // site_settings.key is NULLABLE and the list has no WHERE — a single NULL-key
        // row 500'd the whole endpoint (and get_subdomain/upsert can race a NULL in
        // from a direct write). COALESCE is read-only: no migration, no invented rows.
        sqlx::query_as("SELECT id, COALESCE(key, '') AS key, value::text FROM site_settings ORDER BY key")
            .fetch_all(&state.pool)
            .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|(id, key, raw)| json!({"id": id, "key": key, "value": value_from_text(raw)}))
        .collect();
    Ok(Json(json!(items)))
}

pub async fn get_site_settings(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<Json<Value>> {
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, COALESCE(key, '') AS key, value::text FROM site_settings WHERE key = $1",
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await?;
    let (id, key, raw) = row.ok_or_else(|| AppError::NotFound("Settings not found".into()))?;
    Ok(Json(
        json!({"id": id, "key": key, "value": value_from_text(&raw)}),
    ))
}

pub async fn update_site_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    // Accept both the wrapped form {"value": ...} (what the admin console sends) and a
    // bare JSON body, so old callers keep working.
    let value = match &payload {
        Value::Object(map) if map.len() == 1 && map.contains_key("value") => map["value"].clone(),
        other => other.clone(),
    };
    sqlx::query(
        "INSERT INTO site_settings (id, key, value) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET value = $3",
    )
    .bind(Uuid::new_v4())
    .bind(&slug)
    .bind(text_from_value(&value))
    .execute(&state.pool)
    .await?;
    Ok(Json(
        json!({"message": "Settings updated", "key": slug, "value": value}),
    ))
}
