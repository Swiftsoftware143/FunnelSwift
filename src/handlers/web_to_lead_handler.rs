use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateWebToLeadConfig {
    pub name: String,
    pub form_title: Option<String>,
    pub fields: Option<Vec<String>>,
    pub thank_you_message: Option<String>,
    pub redirect_url: Option<String>,
}

pub async fn list_web_to_lead_configs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // This SELECTed a form_title column that does not exist (and read a timestamptz into a
    // NaiveDateTime) then hid the failure behind `.unwrap_or_default()`, so the list came
    // back empty for every tenant no matter how many forms they had.
    let rows: Vec<(Uuid, String, Value, bool, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, name, field_mapping, is_active, created_at FROM web_to_lead_configs \
         WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, mapping, is_active, created_at)| {
            json!({
                "id": id.to_string(),
                "name": name,
                "form_title": mapping.get("form_title").cloned().unwrap_or(json!("")),
                "fields": mapping.get("fields").cloned().unwrap_or(json!([])),
                "is_active": is_active,
                "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!(out)))
}
pub async fn create_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateWebToLeadConfig>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_forms", "Web-to-lead forms").await?;
    let fields = payload
        .fields
        .unwrap_or_else(|| vec!["name".to_string(), "email".to_string()]);
    // `web_to_lead_configs` has no form_title/fields columns and mints its own uuid
    // public_key — the previous INSERT named two phantom columns and bound a String into
    // a uuid, so every create returned 500 and the product never worked once.
    let field_mapping = json!({
        "form_title": payload.form_title.as_deref().unwrap_or("Get Started"),
        "fields": fields,
    });
    let public_key: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO web_to_lead_configs (id, tenant_id, name, default_source, field_mapping) \
         VALUES ($1, $2, $3, 'Web Form', $4) RETURNING public_key",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&payload.name)
    .bind(&field_mapping)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id.to_string(),
            "public_key": public_key.unwrap_or(id).to_string(),
        })),
    ))
}
pub async fn update_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // This handler used to return {"message":"Config updated"} without touching the
    // database — an edit in the app looked saved and reverted on the next fetch.
    let mut mapping = json!({});
    if let Some(title) = payload.get("form_title") {
        mapping["form_title"] = title.clone();
    }
    if let Some(fields) = payload.get("fields") {
        mapping["fields"] = fields.clone();
    }
    let affected = sqlx::query(
        "UPDATE web_to_lead_configs SET \
           name = COALESCE($3, name), \
           is_active = COALESCE($4, is_active), \
           default_source = COALESCE($5, default_source), \
           field_mapping = field_mapping || $6::jsonb, \
           updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(payload.get("name").and_then(|v| v.as_str()))
    .bind(payload.get("is_active").and_then(|v| v.as_bool()))
    .bind(payload.get("default_source").and_then(|v| v.as_str()))
    .bind(&mapping)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("Config not found".into()));
    }
    Ok(Json(json!({"message": "Config updated"})))
}
pub async fn delete_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("DELETE FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Config deleted"})))
}
pub async fn get_web_to_lead_embed(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let public_key: Option<Uuid> = sqlx::query_scalar(
        "SELECT public_key FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let public_key = public_key.ok_or_else(|| AppError::NotFound("Config not found".into()))?;
    // The public endpoint resolves the tenant from public_key, so a snippet without the key
    // (and pointing at a /embed.js that does not exist) could never capture a lead.
    let embed_code = format!(
        "<form id=\"fsw-wtl-{id}\" onsubmit=\"event.preventDefault();fetch('https://funnelswift.net/api/v1/web-to-lead',{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{public_key:'{public_key}',name:this.name.value,email:this.email.value}})}}).then(()=>this.reset());\">\n  <input name=\"name\" placeholder=\"Name\" required />\n  <input name=\"email\" type=\"email\" placeholder=\"Email\" required />\n  <button type=\"submit\">Send</button>\n</form>"
    );
    Ok(Json(json!({
        "embed_code": embed_code,
        "public_key": public_key.to_string(),
        "endpoint": "https://funnelswift.net/api/v1/web-to-lead",
    })))
}
pub async fn handle_web_to_lead(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Resolve the tenant from the config's public_key — never hardcode or trust a body tenant_id.
    let public_key = payload["public_key"].as_str().unwrap_or("");
    // public_key is a uuid column: binding the raw string made Postgres fail with
    // "operator does not exist: uuid = text", so no form submission ever captured a lead.
    let public_key = Uuid::parse_str(public_key)
        .map_err(|_| AppError::BadRequest("Invalid public_key".into()))?;
    let tenant_id: Option<Uuid> =
        sqlx::query_scalar("SELECT tenant_id FROM web_to_lead_configs WHERE public_key = $1")
            .bind(public_key)
            .fetch_optional(&state.pool)
            .await?;
    let tenant_id = tenant_id.ok_or_else(|| AppError::BadRequest("Unknown public_key".into()))?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO leads (id, tenant_id, name, email, source, status) VALUES ($1, $2, $3, $4, 'web', 'new')")
        .bind(id).bind(tenant_id)
        .bind(payload["name"].as_str().unwrap_or("")).bind(payload["email"].as_str().unwrap_or(""))
        .execute(&state.pool).await?;

    // INBOUND CoreSwift push (fleet standard 2026-09-20 §R2): every captured opt-in must be
    // able to land in CoreSwift as a contact. Fire-and-forget: the visitor's submission above
    // already succeeded, and this is a no-op when the tenant has no `coreswift` BYOK key.
    crate::coreswift::spawn_lead_push(
        state.pool.clone(),
        tenant_id,
        crate::coreswift::LeadPayload {
            email: payload
                .get("email")
                .and_then(|v| v.as_str())
                .map(String::from),
            phone: payload
                .get("phone")
                .and_then(|v| v.as_str())
                .map(String::from),
            name: payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from),
            company: payload
                .get("company")
                .and_then(|v| v.as_str())
                .map(String::from),
            source: Some("web_to_lead".to_string()),
            ..Default::default()
        },
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.to_string(), "message": "Lead captured"})),
    ))
}
