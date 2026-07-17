use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use crate::error::AppError;

// reqwest is used for cross-app sync — already in Cargo.toml dependencies

/// Evaluates tag rules when tags change on a lead.
/// Returns (tags_to_remove, tags_to_add) based on active rules.
pub async fn evaluate_tag_rules(
    pool: &PgPool,
    tenant_id: Uuid,
    current_tags: &[Value],
    newly_assigned_tag_ids: &[Uuid],
) -> std::result::Result<(Vec<String>, Vec<String>), AppError> {
    if newly_assigned_tag_ids.is_empty() {
        return Ok((vec![], vec![]));
    }

    // Get active rules that trigger on any of the newly assigned tags
    let rules = sqlx::query_as::<_, (String, String, Option<Uuid>, Option<String>)>(
        r#"
        SELECT r.action_type, t.name AS trigger_tag_name, r.action_tag_id, at.name AS action_tag_name
        FROM tag_rules r
        JOIN tags t ON t.id = r.trigger_tag_id
        LEFT JOIN tags at ON at.id = r.action_tag_id
        WHERE (r.tenant_id = $1 OR r.tenant_id = '00000000-0000-0000-0000-000000000001')
          AND r.is_active = true
          AND r.trigger_tag_id = ANY($2)
        "#
    )
    .bind(tenant_id)
    .bind(newly_assigned_tag_ids)
    .fetch_all(pool)
    .await?;

    let mut to_remove: Vec<String> = vec![];
    let mut to_add: Vec<String> = vec![];

    // Get current tag names for lookup
    let current_tag_names: Vec<String> = current_tags.iter()
        .filter_map(|t| t.as_str().map(|s| s.to_string()))
        .collect();

    for (action_type, _trigger_name, action_tag_id, action_tag_name) in rules {
        match action_type.as_str() {
            "remove_tag" => {
                if let Some(ref name) = action_tag_name {
                    if current_tag_names.contains(name) && !to_remove.contains(name) {
                        to_remove.push(name.clone());
                    }
                }
            }
            "add_tag" => {
                if let Some(ref name) = action_tag_name {
                    if !current_tag_names.contains(name) && !to_add.contains(name) {
                        to_add.push(name.clone());
                    }
                }
            }
            "replace" => {
                // Remove all current tags, add the action tag
                for t in &current_tag_names {
                    if !to_remove.contains(t) {
                        to_remove.push(t.clone());
                    }
                }
                if let Some(ref name) = action_tag_name {
                    if !to_add.contains(name) {
                        to_add.push(name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    Ok((to_remove, to_add))
}

/// System tag IDs (deterministic namespace-based UUIDs)
pub const SOLD_TAG_ID: &str = "3b008e4a-dbc8-5558-8762-2e1787ec7c2c";
pub const QUALIFIED_TAG_ID: &str = "15698a9a-67fe-5bf1-9aac-1dcd7a1ccd9e";
pub const SOLD_TAG_NAME: &str = "Sold";
pub const QUALIFIED_TAG_NAME: &str = "Qualified";

/// Auto-apply "Sold" tag to all leads in a tenant when the plan is upgraded
/// from free/kinetic_free to pro/enterprise (paid plans).
/// This also triggers the "Sold removes Qualified" tag rule via assign_lead_tags.
pub async fn apply_sold_to_tenant_leads(
    pool: &PgPool,
    tenant_id: Uuid,
    new_plan_slug: &str,
) -> std::result::Result<u64, AppError> {
    // Only apply Sold for paid plans
    let is_paid = matches!(new_plan_slug, "pro" | "enterprise");
    if !is_paid {
        tracing::info!("apply_sold: plan {} is not paid, skipping", new_plan_slug);
        return Ok(0);
    }

    // Verify Sold tag exists
    let sold_tag_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE id = $1::uuid)"
    )
    .bind(Uuid::parse_str(SOLD_TAG_ID).expect(" SOLD_TAG_ID is a valid UUID constant"))
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    if !sold_tag_exists {
        tracing::warn!(
            "Sold system tag (id={}) not found - skipping auto-apply for tenant {}",
            SOLD_TAG_ID, tenant_id
        );
        return Ok(0);
    }

    // Find all leads in this tenant that don't already have Sold
    // and update their tags to include "Sold", then evaluate rules
    let leads: Vec<(Uuid, Option<Value>)> = sqlx::query_as(
        "SELECT id, tags FROM leads WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let mut updated_count = 0u64;
    let sold_tag_name = SOLD_TAG_NAME.to_string();

    for (lead_id, tags_val) in &leads {
        let mut current_tags: Vec<String> = match tags_val {
            Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
            _ => vec![],
        };

        if current_tags.contains(&sold_tag_name) {
            continue; // Already has Sold
        }

        // Add Sold tag
        current_tags.push(sold_tag_name.clone());

        // Evaluate tag rules (this will auto-remove Qualified if the rule is active)
        let newly_assigned_ids = vec![Uuid::parse_str(SOLD_TAG_ID).expect(" SOLD_TAG_ID is a valid UUID constant")];
        let (to_remove, to_add) = evaluate_tag_rules(
            pool,
            tenant_id,
            &current_tags.iter().map(|s| Value::String(s.clone())).collect::<Vec<_>>(),
            &newly_assigned_ids,
        ).await?;

        // Apply rule results
        current_tags.retain(|t| !to_remove.contains(t));
        for t in &to_add {
            if !current_tags.contains(t) {
                current_tags.push(t.clone());
            }
        }

        let tags_json: Value = Value::Array(current_tags.iter().map(|t| Value::String(t.clone())).collect());
        
        sqlx::query("UPDATE leads SET tags = $1::jsonb, updated_at = NOW() WHERE id = $2")
            .bind(&tags_json)
            .bind(lead_id)
            .execute(pool)
            .await?;

        // Log changes
        log_tag_change(pool, tenant_id, *lead_id, &[sold_tag_name.clone()], &to_remove, "plan_upgrade").await?;

        // Fire cross-app sync to CoreSwift
        let cs_url = std::env::var("CORESWIFT_URL").unwrap_or_default();
        let internal_sync_key = std::env::var("INTERNAL_SYNC_KEY").unwrap_or_default();
        tracing::info!("apply_sold: cross-app sync cs_url='{}' key_len={}", cs_url, internal_sync_key.len());
        if !cs_url.is_empty() {
            let lead_info: Option<(String, String, String)> = sqlx::query_as(
                "SELECT COALESCE(name, ''), COALESCE(email, ''), COALESCE(company, '') FROM leads WHERE id = $1"
            )
            .bind(lead_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

            if let Some((lname, lemail, lcompany)) = lead_info {
                let sync_payload = json!({
                    "event": "tag_sync",
                    "source_app": "funnelswift",
                    "tenant_id": tenant_id,
                    "lead": {
                        "id": lead_id,
                        "name": lname,
                        "email": lemail,
                        "company": lcompany
                    },
                    "tags": current_tags,
                    "added_tags": [sold_tag_name.clone()],
                    "removed_tags": to_remove,
                    "triggered_by": "plan_upgrade"
                });

                let url = format!("{}/api/v1/webhooks/cross-app/tag-sync", cs_url);
                tokio::spawn(async move {
                    let client = reqwest::Client::new();
                    let _ = client.post(&url)
                        .header("x-internal-key", &internal_sync_key)
                        .json(&sync_payload)
                        .timeout(std::time::Duration::from_secs(5))
                        .send()
                        .await;
                });
            }
        }

        updated_count += 1;
    }

    tracing::info!(
        "Auto-applied Sold tag to {} leads in tenant {} (upgraded to {})",
        updated_count, tenant_id, new_plan_slug
    );

    Ok(updated_count)
}

/// Logs tag changes for audit trail and cross-app sync
pub async fn log_tag_change(
    pool: &PgPool,
    tenant_id: Uuid,
    lead_id: Uuid,
    added_tags: &[String],
    removed_tags: &[String],
    triggered_by: &str,
) -> std::result::Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO tag_change_log (tenant_id, lead_id, added_tags, removed_tags, triggered_by)
           VALUES ($1, $2, $3::jsonb, $4::jsonb, $5)"#
    )
    .bind(tenant_id)
    .bind(lead_id)
    .bind(serde_json::to_value(added_tags).unwrap_or_default())
    .bind(serde_json::to_value(removed_tags).unwrap_or_default())
    .bind(triggered_by)
    .execute(pool)
    .await?;

    Ok(())
}
