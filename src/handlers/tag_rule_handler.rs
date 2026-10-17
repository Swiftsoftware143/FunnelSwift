use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

/// tag_rules has real columns (name, trigger_tag_id, action_type, action_tag_id, target_app,
/// is_active) but create/update used to answer 201/200 without touching the database, so the
/// API looked healthy while nothing persisted. Both now write, scoped to the caller's tenant.
fn tenant_of(auth: &AuthUser) -> Uuid {
    Uuid::parse_str(&auth.tenant_id).unwrap_or_default()
}

fn opt_uuid(payload: &Value, key: &str) -> Option<Uuid> {
    payload[key].as_str().and_then(|s| Uuid::parse_str(s).ok())
}

/// Column widths these writers bind into, straight from the live schema: `tag_rules.name`
/// `varchar(255)`, `action_type` `varchar(50)`, `target_app` `varchar(100)` (`description` is
/// `text`, so it needs no cap). None of them was checked, so an over-long value reached the
/// INSERT/UPDATE and came back as the raw `500 {"error":"Database error"}` instead of a refusal the
/// console can read — the same hole the tag-name fix closed on `tags`.
const RULE_NAME_CHARS: usize = 255;
const RULE_ACTION_TYPE_CHARS: usize = 50;
const RULE_TARGET_APP_CHARS: usize = 100;

/// Refuse a value the column cannot store, naming the field, so the panel can show the reason.
fn check_storable(value: &str, max_chars: usize, label: &str) -> AppResult<()> {
    crate::security::display_name::check_len(value, max_chars, label)
        .map_err(AppError::UnprocessableEntity)
}

/// The update path only touches the keys it was sent, so check exactly those.
fn check_storable_keys(payload: &Value, keys: &[(&str, usize)]) -> AppResult<()> {
    for (key, max_chars) in keys {
        if let Some(value) = payload[*key].as_str() {
            check_storable(value, *max_chars, key)?;
        }
    }
    Ok(())
}

pub async fn list_tag_rules(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let rows: Vec<(Uuid, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, COALESCE(name,''), created_at FROM tag_rules WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!(rows)))
}

pub async fn create_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = tenant_of(&auth);
    let name = payload["name"].as_str().unwrap_or("").trim().to_string();
    let trigger_tag_id = opt_uuid(&payload, "trigger_tag_id");
    let action_type = payload["action_type"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let trigger_tag_id =
        trigger_tag_id.ok_or_else(|| AppError::BadRequest("trigger_tag_id is required".into()))?;
    if action_type.is_empty() {
        return Err(AppError::BadRequest("action_type is required".into()));
    }
    // Lengths are refused before the write (see `check_storable`): an over-long rule name or
    // action type would otherwise surface as a raw database error from the INSERT.
    check_storable(&name, RULE_NAME_CHARS, "name")?;
    check_storable(&action_type, RULE_ACTION_TYPE_CHARS, "action_type")?;
    let id = Uuid::new_v4();
    let target_app = payload["target_app"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "funnelswift".to_string());
    check_storable(&target_app, RULE_TARGET_APP_CHARS, "target_app")?;
    let is_active = payload["is_active"].as_bool().unwrap_or(true);
    sqlx::query(
        "INSERT INTO tag_rules
           (id, tenant_id, name, description, trigger_tag_id, action_type, action_tag_id, target_app, is_active)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&name)
    .bind(payload["description"].as_str())
    .bind(trigger_tag_id)
    .bind(&action_type)
    .bind(opt_uuid(&payload, "action_tag_id"))
    .bind(&target_app)
    .bind(is_active)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "message": "Tag rule created"})),
    ))
}

pub async fn update_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    // An update writes only the keys it carries, so an over-long value on ANY of them must be
    // refused here rather than becoming a 500 from the UPDATE.
    check_storable_keys(
        &payload,
        &[
            ("name", RULE_NAME_CHARS),
            ("action_type", RULE_ACTION_TYPE_CHARS),
            ("target_app", RULE_TARGET_APP_CHARS),
        ],
    )?;
    let updated: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE tag_rules SET
           name = COALESCE($3, name),
           description = COALESCE($4, description),
           action_type = COALESCE($5, action_type),
           action_tag_id = COALESCE($6, action_tag_id),
           target_app = COALESCE($7, target_app),
           is_active = COALESCE($8, is_active),
           updated_at = NOW()
         WHERE id = $1 AND tenant_id = $2
         RETURNING id",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(payload["name"].as_str())
    .bind(payload["description"].as_str())
    .bind(payload["action_type"].as_str())
    .bind(opt_uuid(&payload, "action_tag_id"))
    .bind(payload["target_app"].as_str())
    .bind(payload["is_active"].as_bool())
    .fetch_optional(&state.pool)
    .await?;
    if updated.is_none() {
        return Err(AppError::NotFound("Tag rule not found".into()));
    }
    Ok(Json(json!({"id": id, "message": "Tag rule updated"})))
}

pub async fn delete_tag_rule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let result = sqlx::query("DELETE FROM tag_rules WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Tag rule not found".into()));
    }
    Ok(Json(json!({"message": "Tag rule deleted"})))
}

/// `GET /api/v1/tag-change-log` — the workspace's own audit trail of tag changes, newest first.
///
/// The table has three real writers (`tag_logic::log_tag_change` from `POST|PUT /api/v1/leads/:id/tags`
/// and the rule engine, plus the set-based insert a system-tag delete runs) and until now it had no
/// reader at all: this handler answered a constant `[]` with `_auth`/`_state` unused, so every change
/// the app recorded was invisible to the operator who made it. It now reads the caller's OWN tenant
/// (never another's), newest first, with the lead the change landed on, and caps the answer at 200
/// rows so a busy workspace cannot be asked to render an unbounded table.
pub async fn list_tag_change_log(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let rows = sqlx::query(
        "SELECT cl.id, cl.lead_id, l.name AS lead_name, l.email AS lead_email, \
                cl.added_tags, cl.removed_tags, cl.triggered_by, cl.created_at \
           FROM tag_change_log cl \
           LEFT JOIN leads l ON l.id = cl.lead_id AND l.tenant_id = cl.tenant_id \
          WHERE cl.tenant_id = $1 \
          ORDER BY cl.created_at DESC \
          LIMIT 200",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid, _>("id").ok(),
                "lead_id": r.try_get::<Uuid, _>("lead_id").ok(),
                "lead_name": r.try_get::<Option<String>, _>("lead_name").unwrap_or(None),
                "lead_email": r.try_get::<Option<String>, _>("lead_email").unwrap_or(None),
                "added_tags": r.try_get::<Option<Value>, _>("added_tags").unwrap_or(None),
                "removed_tags": r.try_get::<Option<Value>, _>("removed_tags").unwrap_or(None),
                "triggered_by": r.try_get::<Option<String>, _>("triggered_by").unwrap_or(None),
                "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").ok(),
            })
        })
        .collect();

    Ok(Json(json!(items)))
}
