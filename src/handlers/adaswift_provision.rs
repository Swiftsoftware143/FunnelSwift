use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Check if a lead has any `adaswift:` tags and auto-provision in ADASwift.
/// Fire-and-forget — never blocks the response.
pub async fn check_and_provision(
    pool: &PgPool,
    adaswift_url: &str,
    lead_id: Uuid,
    tenant_id: Uuid,
) {
    // Fetch the lead's tags
    let row: Result<Option<(Option<Value>,)>, _> = sqlx::query_as(
        "SELECT tags FROM leads WHERE id = $1 AND tenant_id = $2"
    )
    .bind(lead_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await;

    let tags: Vec<String> = match row {
        Ok(Some((Some(Value::Array(arr)),))) => {
            arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
        }
        _ => return,
    };

    // Look for an adaswift: tag
    let provision_tag = tags.iter().find(|t| t.starts_with("adaswift:"));
    let plan_tier = match provision_tag {
        Some(tag) => tag.strip_prefix("adaswift:").unwrap_or("").to_string(),
        None => return,
    };

    if plan_tier.is_empty() {
        return;
    }

    // Get the lead info for provisioning
    let lead_info = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT name, email, company FROM leads WHERE id = $1"
    )
    .bind(lead_id)
    .fetch_optional(pool)
    .await;

    let (name, email, company) = match lead_info {
        Ok(Some((n, e, c))) => (n, e, c),
        _ => return,
    };

    let email = match email {
        Some(e) if !e.is_empty() => e,
        _ => {
            tracing::warn!(%lead_id, "adaswift_provision: lead has no email, skipping");
            return;
        }
    };

    let api_key = std::env::var("INTERNAL_SYNC_KEY").unwrap_or_default();
    if api_key.is_empty() {
        tracing::warn!("adaswift_provision: INTERNAL_SYNC_KEY not set");
        return;
    }

    // Derive domain from company name or use a placeholder
    let domain = company
        .as_deref()
        .unwrap_or(&name)
        .to_lowercase()
        .replace(' ', "")
        .replace(|c: char| !c.is_alphanumeric(), "");

    let payload = serde_json::json!({
        "name": name,
        "email": email,
        "domain": domain,
        "plan_tier": plan_tier,
        "company": company,
        "api_key": api_key,
    });

    let provision_url = format!("{}/api/v1/internal/provision", adaswift_url.trim_end_matches('/'));

    match reqwest::Client::new()
        .post(&provision_url)
        .json(&payload)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                tracing::info!(%lead_id, %status, "adaswift_provision: provisioned");
            } else {
                tracing::warn!(%lead_id, %status, body = %body, "adaswift_provision: non-success");
            }
        }
        Err(e) => {
            tracing::warn!(%lead_id, error = %e, "adaswift_provision: request failed");
        }
    }
}
