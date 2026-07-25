use reqwest::Client;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// CoreSwift tenant UUIDs
const SWIFTSOFTWARE_TENANT: &str = "abd8ad22-aa01-4642-9a9f-6bef6a03d85b";

/// Push new affiliate signup to CoreSwift — add to Affiliates list, tag as "Affiliate"
pub async fn push_affiliate_to_coreswift(
    coreswift_url: &str,
    internal_sync_key: &str,
    affiliate_id: &str,
    email: &str,
    first_name: &str,
    last_name: &str,
    commission_rate: f64,
    selected_apps: &[String],
) {
    if coreswift_url.is_empty() || internal_sync_key.is_empty() {
        tracing::debug!("coreswift=push: skipping affiliate sync (no URL or key)");
        return;
    }

    let client = Client::new();
    let base = coreswift_url.trim_end_matches('/');
    let key = internal_sync_key;

    // Determine tier based on commission rate
    let tier_tag = if commission_rate >= 20.0 { "Affiliate Tier: Platinum" }
        else if commission_rate >= 15.0 { "Affiliate Tier: Gold" }
        else if commission_rate >= 10.0 { "Affiliate Tier: Silver" }
        else { "Affiliate Tier: Standard" };

    // Use the cross-app tag-sync webhook format that CoreSwift natively understands
    let tags: Vec<&str> = vec!["Affiliate", tier_tag, "Source: FunnelSwift"];
    let added_tags: Vec<&str> = vec!["Affiliates"];
    let removed_tags: Vec<&str> = vec![];
    let payload = json!({
        "source_app": "funnelswift",
        "tenant_id": SWIFTSOFTWARE_TENANT,
        "lead": {
            "id": affiliate_id,
            "name": format!("{} {}", first_name, last_name),
            "email": email,
            "company": format!("SwiftSoftware Affiliate — {}% commission — Apps: {}",
                commission_rate, selected_apps.join(", "))
        },
        "tags": tags,
        "added_tags": added_tags,
        "removed_tags": removed_tags,
        "triggered_by": "affiliate_signup"
    });

    let url = format!("{}/api/v1/webhooks/cross-app/tag-sync", base);

    match client
        .post(&url)
        .header("x-internal-key", key)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                tracing::info!(
                    affiliate_id = %affiliate_id,
                    email = %email,
                    tier = %tier_tag,
                    "affiliate pushed to CoreSwift: Affiliates list + Affiliate tag"
                );
            } else {
                tracing::warn!("affiliate coreswift push returned {}: {:?}", resp.status(), resp.text().await);
            }
        }
        Err(e) => {
            tracing::warn!("affiliate coreswift push failed: {e}");
        }
    }
}

/// Push lead to CoreSwift on a best-effort basis (fire-and-forget).
/// Used for lead creation, pipeline stage changes, and tag updates.
pub async fn push_to_coreswift(
    pool: &PgPool,
    coreswift_url: &str,
    internal_sync_key: &str,
    lead_id: Uuid,
    tenant_id: Uuid,
) {
    if coreswift_url.is_empty() {
        tracing::debug!("coreswift_url is empty, skipping push");
        return;
    }

    // Fetch the lead from DB
    let row = match sqlx::query_as::<_, (String, String, String)>(
        "SELECT COALESCE(name, ''), COALESCE(email, ''), COALESCE(company, '') FROM leads WHERE id = $1 AND tenant_id = $2"
    )
    .bind(lead_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!("Lead {lead_id} not found for tenant {tenant_id}, skipping CoreSwift push");
            return;
        }
        Err(e) => {
            tracing::warn!("Failed to fetch lead {lead_id} for CoreSwift push: {e}");
            return;
        }
    };

    let (name, email, company) = row;

    // Get current tags
    let tags: Vec<String> = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT tags FROM leads WHERE id = $1"
    )
    .bind(lead_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
    .and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter().filter_map(|t| t.as_str().map(String::from)).collect()
        })
    })
    .unwrap_or_default();

    let payload = json!({
        "source_app": "funnelswift",
        "tenant_id": "abd8ad22-aa01-4642-9a9f-6bef6a03d85b",   // SwiftSoftware tenant
        "lead": {
            "id": lead_id,
            "name": name,
            "email": email,
            "company": company
        },
        "tags": tags,
        "added_tags": [],
        "removed_tags": [],
        "triggered_by": "lead_sync"
    });

    let url = format!("{}/api/v1/webhooks/cross-app/tag-sync", coreswift_url);
    let client = Client::new();
    let sync_key = internal_sync_key.to_string();

    match client
        .post(&url)
        .header("x-internal-key", &sync_key)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                tracing::debug!("CoreSwift sync OK for lead {}", lead_id);
            } else {
                tracing::warn!("CoreSwift sync returned {} for lead {}", status, lead_id);
            }
        }
        Err(e) => {
            tracing::warn!("CoreSwift sync failed for lead {}: {}", lead_id, e);
        }
    }
}
