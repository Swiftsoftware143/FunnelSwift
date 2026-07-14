use reqwest::Client;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Fetch a lead from the DB and push it to WorkflowSwift on a best-effort basis.
pub async fn push_to_workflowswift(
    pool: &PgPool,
    workflowswift_url: &str,
    lead_id: Uuid,
    aid: Uuid,
) {
    if workflowswift_url.is_empty() {
        tracing::debug!("workflowswift_url is empty, skipping push");
        return;
    }

    // Fetch the full lead from DB
    let lead = match sqlx::query_as::<_, crate::models::lead::Lead>(
        "SELECT * FROM leads WHERE id = $1 AND aid = $2",
    )
    .bind(lead_id)
    .bind(aid)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(lead)) => lead,
        Ok(None) => {
            tracing::warn!("Lead {lead_id} not found for account {aid}, skipping WorkflowSwift push");
            return;
        }
        Err(e) => {
            tracing::warn!("Failed to fetch lead {lead_id} for WorkflowSwift push: {e}");
            return;
        }
    };

    // Parse name into first_name/last_name — best effort
    let (first_name, last_name) = match &lead.name {
        Some(name) if !name.trim().is_empty() => {
            let parts: Vec<&str> = name.trim().splitn(2, ' ').collect();
            let first = parts.first().map(|s| s.to_string()).unwrap_or_default();
            let last = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
            (first, last)
        }
        _ => (String::new(), String::new()),
    };

    let payload = json!({
        "source": "funnelswift",
        "campaign_slug": "funnelswift",
        "contact": {
            "first_name": first_name,
            "last_name": last_name,
            "email": lead.email.as_deref().unwrap_or(""),
            "phone": lead.phone.as_deref().unwrap_or(""),
            "business_name": lead.company.as_deref().unwrap_or("")
        },
        "data": {
            "lead": {
                "id": lead.id,
                "aid": lead.id,
                "name": lead.name,
                "email": lead.email,
                "phone": lead.phone,
                "company": lead.company,
                "status": lead.status,
                "stage": lead.stage,
                "source": lead.source,
                "tags": lead.tags,
                "notes": lead.notes,
                "assigned_to": lead.assigned_to,
                "score": lead.score,
                "custom_fields": lead.custom_fields,
                "created_at": lead.created_at,
                "updated_at": lead.updated_at
            },
            "source": lead.source.as_deref().unwrap_or("funnelswift"),
            "stage": lead.stage.as_deref().unwrap_or(""),
            "custom_fields": lead.custom_fields
        },
        "source_entry_id": lead_id
    });

    let url = format!("{}/api/incoming", workflowswift_url.trim_end_matches('/'));

    let internal_key = std::env::var("INTERNAL_SYNC_KEY").unwrap_or_default();
    let client = Client::new();
    let mut req = client.post(&url).json(&payload);
    if !internal_key.is_empty() {
        req = req.header("X-Internal-Key", &internal_key);
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                tracing::info!("Successfully pushed lead {lead_id} to WorkflowSwift");
            } else {
                let body = resp.text().await.unwrap_or_else(|e| format!("(failed to read body: {e})"));
                tracing::warn!("WorkflowSwift returned {status} for lead {lead_id}: {body}");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to push lead {lead_id} to WorkflowSwift at {url}: {e}");
        }
    }
}
