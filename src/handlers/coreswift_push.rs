// Coreswift push handler — auto-syncs leads to CoreSwift CRM
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use uuid::Uuid;

/// Called internally by auth handler on user registration or lead creation
/// Pushes user/lead info to CoreSwift CRM tag-provision endpoint
/// Returns Ok(()) on success, Err(message) on failure
pub async fn push_to_coreswift(
    coreswift_url: &str,
    internal_key: &str,
    _tenant_id: Uuid,
    lead_name: &str,
    lead_email: &str,
    lead_company: Option<&str>,
) -> Result<(), String> {
    if coreswift_url.is_empty() || internal_key.is_empty() {
        return Err("coreswift_url or internal_key not configured".into());
    }

    tracing::info!(
        "push_to_coreswift: syncing {} <{}> to CoreSwift CRM",
        lead_name,
        lead_email
    );

    let last_name: String = lead_name
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    let company = lead_company.unwrap_or("");
    let payload = serde_json::json!({
        "source": "funnelswift",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "contact": {
            "id": null,
            "first_name": lead_name.split_whitespace().next().unwrap_or("User"),
            "last_name": if last_name.is_empty() { "" } else { &last_name },
            "email": lead_email,
            "phone": null,
            "company": company,
            "custom_fields": null
        },
        "tag": {
            "name": "funnelswift-lead",
            "campaign_id": null,
            "metadata": null
        }
    });

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/internal/tag-provision",
        coreswift_url.trim_end_matches('/')
    );

    match client
        .post(&url)
        .header("x-internal-key", internal_key)
        .header("Content-Type", "application/json")
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                return Ok(());
            } else {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("CoreSwift returned {}: {}", status, body));
            }
        }
        Err(e) => {
            return Err(format!("Failed to reach CoreSwift: {}", e));
        }
    }
}

/// Push a lead to CoreSwift CRM — API endpoint (manual trigger)
pub async fn push_lead_to_coreswift(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("New Lead");
    let _tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| crate::error::AppError::BadRequest("Invalid tenant".into()))?;
    let coreswift_url = state.coreswift_url.clone();
    let internal_key = state.internal_sync_key.clone();

    if coreswift_url.is_empty() {
        return Ok(Json(
            json!({"status":"skipped","message":"CoreSwift URL not configured"}),
        ));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/internal/tag-provision",
        coreswift_url.trim_end_matches('/')
    );

    let sync_payload = serde_json::json!({
        "source": "funnelswift",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "contact": {
            "id": null,
            "first_name": name.split_whitespace().next().unwrap_or("User"),
            "last_name": name.split_whitespace().skip(1).collect::<Vec<_>>().join(" "),
            "email": email,
            "phone": null,
            "company": payload.get("company").and_then(|v| v.as_str()).unwrap_or(""),
            "custom_fields": null
        },
        "tag": {
            "name": "funnelswift-lead",
            "campaign_id": null,
            "metadata": null
        }
    });

    match client
        .post(&url)
        .header("x-internal-key", &internal_key)
        .header("Content-Type", "application/json")
        .json(&sync_payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(json!({}));
            if status.is_success() {
                let cid = body
                    .get("contact_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| body.get("id").and_then(|v| v.as_str()))
                    .unwrap_or("");
                let tid = body.get("tenant_id").and_then(|v| v.as_str()).unwrap_or("");
                Ok(Json(
                    json!({"status":"pushed","coreswift_contact_id":cid,"coreswift_tenant_id":tid,"message":format!("Lead '{}' pushed to CoreSwift CRM",name)}),
                ))
            } else {
                let err_msg = body
                    .get("error")
                    .and_then(|v| v.as_str())
                    .or_else(|| body.get("message").and_then(|v| v.as_str()))
                    .unwrap_or("CoreSwift rejected the push");
                Ok(Json(
                    json!({"status":"error","coreswift_status":status.as_u16(),"message":err_msg}),
                ))
            }
        }
        Err(e) => Ok(Json(
            json!({"status":"error","message":format!("Failed to reach CoreSwift CRM: {}", e)}),
        )),
    }
}

/// Provision a new CoreSwift user account — API endpoint
pub async fn provision_coreswift_user(
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let email = payload["email"].as_str().unwrap_or("");
    let name = payload["name"].as_str().unwrap_or("User");
    let plan = payload["plan"].as_str().unwrap_or("free");
    let coreswift_url = state.coreswift_url.clone();
    let internal_key = state.internal_sync_key.clone();

    if email.is_empty() {
        return Err(crate::error::AppError::BadRequest(
            "email is required".into(),
        ));
    }
    if coreswift_url.is_empty() {
        return Ok(Json(
            json!({"status":"skipped","message":"CoreSwift URL not configured"}),
        ));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/admin/portfolio-sync",
        coreswift_url.trim_end_matches('/')
    );

    let sync_payload = serde_json::json!({
        "name": name,
        "email": email,
        "description": format!("Provisioned from FunnelSwift — plan: {}", plan),
    });

    match client
        .post(&url)
        .header("x-internal-key", &internal_key)
        .header("Content-Type", "application/json")
        .json(&sync_payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(json!({}));
            if status.is_success() {
                Ok(Json(
                    json!({"status":"provisioned","email":email,"plan":plan,"coreswift_tenant_id":body.get("tenant_id").and_then(|v| v.as_str()).unwrap_or("")}),
                ))
            } else {
                Ok(Json(
                    json!({"status":"error","coreswift_status":status.as_u16(),"message":body.get("message").and_then(|v| v.as_str()).unwrap_or("CoreSwift rejected the provision")}),
                ))
            }
        }
        Err(e) => Ok(Json(
            json!({"status":"error","message":format!("Failed to reach CoreSwift CRM: {}", e)}),
        )),
    }
}

/// Sync a tag to CoreSwift — API endpoint
pub async fn sync_coreswift_tag(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tag = payload["tag"].as_str().unwrap_or("");
    let coreswift_url = state.coreswift_url.clone();
    let internal_key = state.internal_sync_key.clone();

    if tag.is_empty() {
        return Err(crate::error::AppError::BadRequest("tag is required".into()));
    }
    if coreswift_url.is_empty() {
        return Ok(Json(
            json!({"status":"skipped","message":"CoreSwift URL not configured"}),
        ));
    }

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/webhooks/cross-app/tag-sync",
        coreswift_url.trim_end_matches('/')
    );

    let sync_payload = serde_json::json!({
        "event": "tag_sync",
        "source_app": "funnelswift",
        "tags": [tag],
        "tenant_id": auth.tenant_id,
    });

    match client
        .post(&url)
        .header("x-internal-key", &internal_key)
        .header("Content-Type", "application/json")
        .json(&sync_payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(Json(
                    json!({"status":"synced","tag":tag,"message":format!("Tag '{}' synced to CoreSwift",tag)}),
                ))
            } else {
                Ok(Json(
                    json!({"status":"error","message":format!("CoreSwift returned status {}", status.as_u16())}),
                ))
            }
        }
        Err(e) => Ok(Json(
            json!({"status":"error","message":format!("Failed to reach CoreSwift CRM: {}", e)}),
        )),
    }
}

/// Check CoreSwift health
pub async fn coreswift_health(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let coreswift_url = state.coreswift_url.clone();

    if coreswift_url.is_empty() {
        return Ok(Json(
            json!({"connected":false,"url":null,"status":"not configured"}),
        ));
    }

    let client = reqwest::Client::new();
    let url = format!("{}/api/health", coreswift_url.trim_end_matches('/'));

    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok(Json(
                    json!({"connected":true,"url":coreswift_url,"status":"healthy"}),
                ))
            } else {
                Ok(Json(
                    json!({"connected":false,"url":coreswift_url,"status":format!("status {}", resp.status().as_u16())}),
                ))
            }
        }
        Err(e) => Ok(Json(
            json!({"connected":false,"url":coreswift_url,"status":format!("unreachable: {}", e)}),
        )),
    }
}
