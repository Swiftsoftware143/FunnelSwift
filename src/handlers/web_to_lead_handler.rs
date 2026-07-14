use argon2::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct WebToLeadRequest {
    pub api_key: String,
    pub config_id: Option<String>,
    pub name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct WebToLeadConfig {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub is_active: bool,
    pub tag_ids: Option<Vec<Uuid>>,
    pub default_source: Option<String>,
    pub field_mapping: serde_json::Value,
    pub allowed_domains: Vec<String>,
    pub rate_limit_per_hour: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWebToLeadConfig {
    pub name: Option<String>,
    pub tag_ids: Option<Vec<Uuid>>,
    pub default_source: Option<String>,
    pub field_mapping: Option<serde_json::Value>,
    pub allowed_domains: Option<Vec<String>>,
    pub rate_limit_per_hour: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWebToLeadConfig {
    pub name: Option<String>,
    pub is_active: Option<bool>,
    pub tag_ids: Option<Vec<Uuid>>,
    pub default_source: Option<String>,
    pub field_mapping: Option<serde_json::Value>,
    pub allowed_domains: Option<Vec<String>>,
    pub rate_limit_per_hour: Option<i32>,
}

// ── GET /api/v1/web-to-lead/configs ──
pub async fn list_web_to_lead_configs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<WebToLeadConfig>>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let rows = sqlx::query(
        "SELECT id, tenant_id, name, is_active, tag_ids, default_source, field_mapping, allowed_domains, rate_limit_per_hour, created_at, updated_at FROM web_to_lead_configs WHERE tenant_id = $1 ORDER BY created_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let configs: Vec<WebToLeadConfig> = rows.iter().map(|r| WebToLeadConfig {
        id: r.try_get("id").unwrap_or_default(),
        tenant_id: r.try_get("tenant_id").unwrap_or_default(),
        name: r.try_get("name").unwrap_or_default(),
        is_active: r.try_get("is_active").unwrap_or(false),
        tag_ids: r.try_get("tag_ids").ok(),
        default_source: r.try_get("default_source").ok(),
        field_mapping: r.try_get("field_mapping").unwrap_or(serde_json::Value::Null),
        allowed_domains: r.try_get::<Vec<String>, _>("allowed_domains").unwrap_or_default(),
        rate_limit_per_hour: r.try_get("rate_limit_per_hour").unwrap_or(100),
        created_at: r.try_get("created_at").unwrap_or_default(),
        updated_at: r.try_get("updated_at").unwrap_or_default(),
    }).collect();

    Ok(Json(configs))
}

// ── POST /api/v1/web-to-lead/configs ──
pub async fn create_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateWebToLeadConfig>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let config_id = Uuid::new_v4();
    let name = req.name.unwrap_or_else(|| "Default Form".to_string());

    sqlx::query(
        r#"INSERT INTO web_to_lead_configs (id, tenant_id, name, tag_ids, default_source, field_mapping, allowed_domains, rate_limit_per_hour)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
    )
    .bind(config_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(&req.tag_ids)
    .bind(&req.default_source)
    .bind(&req.field_mapping)
    .bind(&req.allowed_domains.unwrap_or_default())
    .bind(req.rate_limit_per_hour.unwrap_or(100))
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "id": config_id,
        "name": name,
        "message": "Web-to-lead config created"
    }))))
}

// ── PUT /api/v1/web-to-lead/configs/:id ──
pub async fn update_web_to_lead_config(
    auth: AuthUser,
    Path(config_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<UpdateWebToLeadConfig>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2"
    )
    .bind(config_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    if existing == 0 {
        return Err(AppError::NotFound("Web-to-lead config not found".into()));
    }

    if let Some(name) = req.name {
        sqlx::query("UPDATE web_to_lead_configs SET name = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(&name).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }
    if let Some(is_active) = req.is_active {
        sqlx::query("UPDATE web_to_lead_configs SET is_active = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(is_active).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }
    if let Some(tag_ids) = req.tag_ids {
        sqlx::query("UPDATE web_to_lead_configs SET tag_ids = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(&tag_ids).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }
    if let Some(src) = req.default_source {
        sqlx::query("UPDATE web_to_lead_configs SET default_source = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(&src).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }
    if let Some(domains) = req.allowed_domains {
        sqlx::query("UPDATE web_to_lead_configs SET allowed_domains = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(&domains).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }
    if let Some(rl) = req.rate_limit_per_hour {
        sqlx::query("UPDATE web_to_lead_configs SET rate_limit_per_hour = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(rl).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }
    if let Some(fm) = req.field_mapping {
        sqlx::query("UPDATE web_to_lead_configs SET field_mapping = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(&fm).bind(config_id).bind(tenant_id).execute(&state.pool).await?;
    }

    Ok(Json(serde_json::json!({"message": "Config updated"})))
}

// ── DELETE /api/v1/web-to-lead/configs/:id ──
pub async fn delete_web_to_lead_config(
    auth: AuthUser,
    Path(config_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let deleted = sqlx::query("DELETE FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2")
        .bind(config_id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if deleted == 0 {
        return Err(AppError::NotFound("Web-to-lead config not found".into()));
    }

    Ok(Json(serde_json::json!({"message": "Config deleted"})))
}

// ── GET /api/v1/web-to-lead/configs/:id/embed ──
pub async fn get_web_to_lead_embed(
    auth: AuthUser,
    Path(config_id): Path<Uuid>,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let row = sqlx::query(
        "SELECT id, name, public_key FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2"
    )
    .bind(config_id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Config not found".into()))?;

    let id: Uuid = row.try_get("id")?;
    let name: String = row.try_get("name")?;
    let public_key: Uuid = row.try_get("public_key")?;

    // Script that auto-fills the public key — no manual API key editing needed
    let embed_default = format!(
        r#"<!-- FunnelSwift Web-to-Lead - paste into <head> or before </body> -->
<script>
window.FunnelSwiftConfig = {{
  configId: "{}"
}};
</script>
<script src="https://funnelswift.net/funnelswift-capture.js" defer></script>"#,
        id
    );

    // Alternative: self-contained script tag for footer/body
    let embed_footer = format!(
        r#"<!-- FunnelSwift Web-to-Lead - paste just before </body> -->
<script src="https://funnelswift.net/funnelswift-capture.js" defer
  data-config-id="{}"
  data-public-key="{}"></script>"#,
        id, public_key
    );

    Ok(Json(serde_json::json!({
        "id": id,
        "name": name,
        "public_key": public_key.to_string(),
        "embed_html": embed_default,
        "embed_footer": embed_footer,
        "script_url": "https://funnelswift.net/funnelswift-capture.js",
        "endpoint_url": "https://funnelswift.net/api/v1/web-to-lead",
        "instructions": {
            "header": "Paste in <head> (with defer tag): Best for site-wide capture. Loads in background without slowing your page.",
            "footer": "Paste just before </body>: Guarantees your form DOM is fully loaded before the script runs. Most reliable.",
            "next_to_form": "Paste the script block directly below your <form> tag if you only want it on one page.",
            "wordpress": "Install WPCode plugin -> Add New -> 'Header & Footer' -> paste in Header or Footer section.",
            "webflow": "Site Settings -> Custom Code -> paste in 'Head Code' or 'Footer Code'.",
            "squarespace": "Settings -> Advanced -> Code Injection -> paste in Header or Footer.",
            "wix": "Settings -> Custom Code -> + Add Custom Code -> paste script -> select 'All Pages'."
        }
    })))
}

// ── POST /api/v1/web-to-lead — public endpoint ──
// Accepts: api_key (standard auth) OR config_id only (public_key lookup via config)
pub async fn handle_web_to_lead(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WebToLeadRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    // ── Auth: determine tenant_id via api_key hash OR public_key lookup ──
    let (tenant_id, config_id) = if !req.api_key.is_empty() {
        // Traditional API key auth
        let prefix = if req.api_key.len() >= 8 {
            req.api_key[..8].to_string()
        } else {
            return Err(AppError::BadRequest("Invalid API key format".into()));
        };

        let key_row = sqlx::query(
            "SELECT id, tenant_id, user_id, key_hash, permissions, is_active FROM api_keys WHERE prefix = $1 AND is_active = true"
        )
        .bind(&prefix)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid API key".into()))?;

        let key_id: Uuid = key_row.try_get("id")?;
        let key_tenant: Uuid = key_row.try_get("tenant_id")?;
        let _user_id: Uuid = key_row.try_get("user_id")?;
        let stored_hash: String = key_row.try_get("key_hash")?;
        let permissions: serde_json::Value = key_row.try_get("permissions")?;

        let parsed_hash = PasswordHash::new(&stored_hash)
            .map_err(|_| AppError::Unauthorized("Invalid key hash".into()))?;

        Argon2::default()
            .verify_password(req.api_key.as_bytes(), &parsed_hash)
            .map_err(|_| AppError::Unauthorized("API key verification failed".into()))?;

        let allowed = permissions.get("web_to_lead").and_then(|v| v.as_bool()).unwrap_or(true);
        if !allowed {
            return Err(AppError::Forbidden("API key does not have web-to-lead permission".into()));
        }

        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(key_id)
            .execute(&state.pool)
            .await?;

        let cid = if let Some(ref cid) = req.config_id {
            Uuid::parse_str(cid).ok()
        } else {
            None
        };

        (key_tenant, cid)
    } else {
        // Public key lookup — no API key needed, just config_id
        let cid = if let Some(ref cid) = req.config_id {
            match Uuid::parse_str(cid) {
                Ok(id) => {
                    // Look up the config to get tenant_id
                    let config = sqlx::query(
                        "SELECT tenant_id FROM web_to_lead_configs WHERE id = $1 AND is_active = true"
                    )
                    .bind(id)
                    .fetch_optional(&state.pool)
                    .await?
                    .ok_or_else(|| AppError::Unauthorized("Invalid or inactive web-to-lead config".into()))?;
                    let ten: Uuid = config.try_get("tenant_id")?;
                    (ten, Some(id))
                }
                Err(_) => return Err(AppError::BadRequest("Invalid config_id format".into())),
            }
        } else {
            return Err(AppError::BadRequest("config_id required when api_key is empty".into()));
        };

        (cid.0, cid.1)
    };

    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .or_else(|| headers.get("referer").and_then(|v| v.to_str().ok()))
        .map(|s| s.to_string());

    // ── Rate limiting ──
    let hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    let recent_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM web_to_lead_logs WHERE tenant_id = $1 AND created_at > $2 AND status != 'rejected'"
    )
    .bind(tenant_id)
    .bind(hour_ago)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let rate_limit = 100;
    if recent_count >= rate_limit as i64 {
        let log_id = Uuid::new_v4();
        let _ = sqlx::query(
            "INSERT INTO web_to_lead_logs (id, config_id, tenant_id, requester_ip, origin_domain, status, error_message) VALUES ($1, $2, $3, $4, $5, 'rejected', 'Rate limit exceeded')"
        )
        .bind(log_id)
        .bind(config_id)
        .bind(tenant_id)
        .bind(headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()))
        .bind(&origin)
        .execute(&state.pool)
        .await;

        return Err(AppError::BadRequest("Rate limit exceeded. Try again later.".into()));
    }

    // ── Fetch config to get tag_ids ──
    let config_tag_ids: Option<Vec<Uuid>> = if let Some(cid) = config_id {
        sqlx::query_scalar::<_, Vec<Uuid>>(
            "SELECT COALESCE(tag_ids, '{}') FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2 AND is_active = true"
        )
        .bind(cid)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
    } else {
        None
    };

    // ── Duplicate check ──
    if let Some(ref email) = req.email {
        if !email.trim().is_empty() {
            let existing: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM leads WHERE email = $1 AND tenant_id = $2)"
            )
            .bind(email)
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(false);

            if existing {
                let log_id = Uuid::new_v4();
                let _ = sqlx::query(
                    "INSERT INTO web_to_lead_logs (id, config_id, tenant_id, requester_ip, origin_domain, field_count, status, error_message) VALUES ($1, $2, $3, $4, $5, $6, 'duplicate', 'Email already exists')"
                )
                .bind(log_id)
                .bind(config_id)
                .bind(tenant_id)
                .bind(headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()))
                .bind(&origin)
                .bind(6)
                .execute(&state.pool)
                .await;

                return Ok((StatusCode::OK, Json(serde_json::json!({
                    "success": true,
                    "lead_id": null,
                    "message": "A lead with this email already exists in your workspace"
                }))));
            }
        }
    }

    // ── Build lead ──
    let lead_id = Uuid::new_v4();
    let name = req.name.or_else(|| {
        let first = req.first_name.as_deref().unwrap_or("");
        let last = req.last_name.as_deref().unwrap_or("");
        if first.is_empty() && last.is_empty() {
            None
        } else {
            Some(format!("{} {}", first, last).trim().to_string())
        }
    });

    let source = req.extra.get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("Web Form")
        .to_string();

    let website = req.extra.get("website").and_then(|v| v.as_str()).map(|s| s.to_string());
    let address = req.extra.get("address").and_then(|v| v.as_str()).map(|s| s.to_string());
    let title = req.extra.get("title").or_else(|| req.extra.get("position")).and_then(|v| v.as_str()).map(|s| s.to_string());
    let social = req.extra.get("social").or_else(|| req.extra.get("linkedin")).and_then(|v| v.as_str()).map(|s| s.to_string());

    let mut custom_fields = serde_json::Map::new();
    if let Some(w) = website { custom_fields.insert("website".into(), serde_json::Value::String(w)); }
    if let Some(a) = address { custom_fields.insert("address".into(), serde_json::Value::String(a)); }
    if let Some(t) = title { custom_fields.insert("title".into(), serde_json::Value::String(t)); }
    if let Some(s) = social { custom_fields.insert("social".into(), serde_json::Value::String(s)); }

    let custom_fields_json: Option<serde_json::Value> = if custom_fields.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(custom_fields))
    };

    // ── Build tags JSON array from config's tag_ids ──
    let tags_json: Option<serde_json::Value> = config_tag_ids.as_ref().map(|ids| {
        serde_json::Value::Array(ids.iter().map(|id| serde_json::Value::String(id.to_string())).collect())
    });

    sqlx::query(
        r#"INSERT INTO leads (id, tenant_id, first_name, last_name, name, email, phone, company, source, stage, tags, custom_fields, notes, assigned_to)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'New', $10, $11, $12, $13)"#,
    )
    .bind(lead_id)
    .bind(tenant_id)
    .bind(&req.first_name)
    .bind(&req.last_name)
    .bind(&name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.company)
    .bind(&source)
    .bind(&tags_json)
    .bind(&custom_fields_json)
    .bind(&req.notes)
    .bind(Option::<Uuid>::None)
    .execute(&state.pool)
    .await?;

    let log_id = Uuid::new_v4();
    let _ = sqlx::query(
        "INSERT INTO web_to_lead_logs (id, config_id, tenant_id, requester_ip, origin_domain, field_count, lead_id, status) VALUES ($1, $2, $3, $4, $5, $6, $7, 'imported')"
    )
    .bind(log_id)
    .bind(config_id)
    .bind(tenant_id)
    .bind(headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()))
    .bind(&origin)
    .bind(6)
    .bind(lead_id)
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "success": true,
        "lead_id": lead_id.to_string(),
        "message": "Lead captured successfully"
    }))))
}
