use crate::error::AppError;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

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
        WHERE (r.tenant_id = $1 OR r.tenant_id = $3)
          AND r.is_active = true
          AND r.trigger_tag_id = ANY($2)
        "#
    )
    .bind(tenant_id)
    .bind(newly_assigned_tag_ids)
    // $3 — the fleet's system tenant (its rules apply to every workspace). BOUND, never a UUID
    // literal in the SQL text: see crate::system_tenant (kanban t_92ce05b3).
    .bind(crate::system_tenant::system_tenant_id())
    .fetch_all(pool)
    .await?;

    let mut to_remove: Vec<String> = vec![];
    let mut to_add: Vec<String> = vec![];

    // Get current tag names for lookup
    let current_tag_names: Vec<String> = current_tags
        .iter()
        .filter_map(|t| t.as_str().map(|s| s.to_string()))
        .collect();

    for (action_type, _trigger_name, _action_tag_id, action_tag_name) in rules {
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

/// Auto-apply the "Sold" tag to all leads in a tenant when the plan the tenant was just put on is
/// a PAID one, which in turn triggers the "Sold removes Qualified" tag rule via assign_lead_tags.
///
/// The paid test is `plans.price > 0` — the PLAN ROW, not a slug literal (kanban t_3641f326).
/// It used to be `matches!(new_plan_slug, "pro" | "enterprise")`, a vocabulary that exists only on
/// a from-zero install: live's slugs are capture-free / kinetic-free / capture-starter /
/// kinetic-pro / suite / agency, so on live the test could never be true and no upgrade ever
/// reached this branch. `plans.price` is `double precision NOT NULL DEFAULT 0` and is what the
/// operator actually prices the tier at (measured 2026-10-02: live 9/9/29/79, from-zero 29/79/199
/// for the paid tiers), so it is true on BOTH databases.
pub async fn apply_sold_to_tenant_leads(
    pool: &PgPool,
    tenant_id: Uuid,
    new_plan_id: Uuid,
) -> std::result::Result<u64, AppError> {
    // The caller (`set_active_plan`) has already proven this plan exists, so a miss here is a
    // real anomaly: treat it as unpaid and say so rather than assuming a paid tier.
    let plan: Option<(String, f64)> =
        sqlx::query_as("SELECT slug, COALESCE(price, 0)::float8 FROM plans WHERE id = $1")
            .bind(new_plan_id)
            .fetch_optional(pool)
            .await?;
    let (new_plan_slug, new_plan_price) = match plan {
        Some(row) => row,
        None => {
            tracing::warn!(
                plan = %new_plan_id,
                tenant = %tenant_id,
                "apply_sold: plan row not found — treating the upgrade as unpaid and applying no Sold tag"
            );
            return Ok(0);
        }
    };

    let is_paid = new_plan_price > 0.0;
    if !is_paid {
        tracing::info!(
            "apply_sold: plan {} (price {}) is not paid, skipping",
            new_plan_slug,
            new_plan_price
        );
        return Ok(0);
    }

    // Verify Sold tag exists
    let sold_tag_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE id = $1::uuid)")
            .bind(Uuid::parse_str(SOLD_TAG_ID).expect(" SOLD_TAG_ID is a valid UUID constant"))
            .fetch_one(pool)
            .await
            .unwrap_or(false);

    if !sold_tag_exists {
        tracing::warn!(
            "Sold system tag (id={}) not found - skipping auto-apply for tenant {}",
            SOLD_TAG_ID,
            tenant_id
        );
        return Ok(0);
    }

    // Find all leads in this tenant that don't already have Sold
    // and update their tags to include "Sold", then evaluate rules
    let leads: Vec<(Uuid, Option<Value>)> =
        sqlx::query_as("SELECT id, tags FROM leads WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_all(pool)
            .await?;

    let mut updated_count = 0u64;
    let sold_tag_name = SOLD_TAG_NAME.to_string();

    for (lead_id, tags_val) in &leads {
        let mut current_tags: Vec<String> = match tags_val {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => vec![],
        };

        if current_tags.contains(&sold_tag_name) {
            continue; // Already has Sold
        }

        // Add Sold tag
        current_tags.push(sold_tag_name.clone());

        // Evaluate tag rules (this will auto-remove Qualified if the rule is active)
        let newly_assigned_ids =
            vec![Uuid::parse_str(SOLD_TAG_ID).expect(" SOLD_TAG_ID is a valid UUID constant")];
        let (to_remove, to_add) = evaluate_tag_rules(
            pool,
            tenant_id,
            &current_tags
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect::<Vec<_>>(),
            &newly_assigned_ids,
        )
        .await?;

        // Apply rule results
        current_tags.retain(|t| !to_remove.contains(t));
        for t in &to_add {
            if !current_tags.contains(t) {
                current_tags.push(t.clone());
            }
        }

        let tags_json: Value = Value::Array(
            current_tags
                .iter()
                .map(|t| Value::String(t.clone()))
                .collect(),
        );

        sqlx::query("UPDATE leads SET tags = $1::jsonb, updated_at = NOW() WHERE id = $2")
            .bind(&tags_json)
            .bind(lead_id)
            .execute(pool)
            .await?;

        // Log changes
        log_tag_change(
            pool,
            tenant_id,
            *lead_id,
            std::slice::from_ref(&sold_tag_name),
            &to_remove,
            "plan_upgrade",
        )
        .await?;

        // Fire cross-app sync to CoreSwift
        let cs_url = std::env::var("CORESWIFT_URL").unwrap_or_default();
        let internal_sync_key = std::env::var("INTERNAL_SYNC_KEY").unwrap_or_default();
        tracing::info!(
            "apply_sold: cross-app sync cs_url='{}' key_len={}",
            cs_url,
            internal_sync_key.len()
        );
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
                    let _ = client
                        .post(&url)
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
        updated_count,
        tenant_id,
        new_plan_slug
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
           VALUES ($1, $2, $3::jsonb, $4::jsonb, $5)"#,
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

/// Attribute affiliate commissions when a lead is tagged with a system tag that is
/// linked to an affiliate product (the tag-based routing signal).
///
/// David's model: affiliate products = the SwiftSoftware products; a system tag points at one.
/// When a lead flowing through FunnelSwift gets that tag, record a pending commission
/// linking the referring affiliate → lead → product. The actual amount is filled in
/// later when the upsell happens inside the respective app (the tag is always the
/// free-plan connection).
pub async fn attribute_affiliate_on_tags(
    pool: &PgPool,
    lead_id: Uuid,
    newly_assigned_tag_ids: &[Uuid],
) -> std::result::Result<(), AppError> {
    if newly_assigned_tag_ids.is_empty() {
        return Ok(());
    }

    // Resolve the user account the lead flowed through (the affiliate attribution anchor).
    let created_by: Option<Uuid> = sqlx::query_scalar("SELECT created_by FROM leads WHERE id = $1")
        .bind(lead_id)
        .fetch_optional(pool)
        .await?
        .flatten();
    let Some(user_id) = created_by else {
        return Ok(());
    };

    // Resolve the affiliate for that user account — only if they signed up for the
    // affiliate program (admin / non-affiliate users have no affiliate record → no attribution).
    let affiliate_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM affiliates WHERE user_id = $1 AND is_active = true LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let Some(affiliate_id) = affiliate_id else {
        return Ok(());
    };

    // Find active affiliate products linked to any of the newly-assigned tags.
    //
    // THE DECISION (kanban t_c06d643c, recorded here because this query is the call site that makes
    // the tag vocabulary load-bearing): the tag vocabulary IS product-owned platform seed data, so
    // it was RESTORED under the System tenant by migration 0060 rather than the reader being
    // deleted. Evidence for the restore arm: every tag this query can act on is declared by a
    // migration (000001_initial.sql:263-285 the 22 shared tags, 043:13-39 the seven per-app Free
    // tags, 0014:5-12 Sold/Qualified), each Free tag is the routing signal for exactly one app's
    // free tier, and deleting the System tenant CASCADE-deleted the whole set (tags.is_system = 0 on
    // a table of 5 rows, every affiliate_products.system_tag_id NULL) which is why this SELECT
    // resolved zero products and recorded no commission for any tag. 0060 restores the taxonomy and
    // links each product to its tag, which is the state this query needs; the sibling reader
    // affiliate_tracking_handler::handle_affiliate_upgrade_event resolves by source_app instead.
    // Whoever changes this predicate is changing the affiliate money path - keep the link and this
    // reader in step, and keep `is_active = true` here (a retired product stops being attributable).
    let product_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM affiliate_products WHERE system_tag_id = ANY($1) AND is_active = true",
    )
    .bind(newly_assigned_tag_ids)
    .fetch_all(pool)
    .await?;

    for product_id in product_ids {
        // At most ONE commission row per (lead, product) — the row is never duplicated, and what
        // happens when the tag is applied AGAIN depends on the STATUS that row is already in
        // (kanban t_769b93ba):
        //
        //   pending  — the attribution already stands; skip. Unchanged, and it is what makes the same
        //              tag applied twice still leave exactly one row (t_d4ce013c leg C).
        //   reversed — there are TWO ways a row gets here and they must NOT be conflated:
        //                * `reverse_affiliate_on_tags` below withdrew it because the tag that
        //                  produced it came OFF the lead. It marks the row
        //                  `metadata->>'reversed_by' = 'tag_removed'`, and for THAT row re-applying
        //                  the tag is the operator asking for the attribution back, so the row is
        //                  RE-OPENED to pending (and the mark is consumed).
        //                * `plan_movement.rs:189` withdrew it because the customer LEFT a paying
        //                  plan. That one stays reversed: a tag coming back is not a payment, and
        //                  re-opening it would invent money the customer is not paying. Only the
        //                  authoritative money event un-reverses this one (a real conversion sets
        //                  `earned` and clears `reversed_at`, `plan_movement.rs:172`).
        //              Either way it is the SAME row, never a second one; its affiliate, amount and
        //              metadata are kept.
        //   earned / paid — settled money. The authoritative money event is the conversion webhook,
        //              not the tag, so a re-tag never rewrites or re-opens it.
        let status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM affiliate_commissions WHERE lead_id = $1 AND product_id = $2 LIMIT 1",
        )
        .bind(lead_id)
        .bind(product_id)
        .fetch_optional(pool)
        .await?;
        match status.as_deref() {
            Some("reversed") => {
                let reopened = sqlx::query(
                    "UPDATE affiliate_commissions
                        SET status = 'pending', reversed_at = NULL,
                            metadata = metadata - 'reversed_by'
                      WHERE lead_id = $1 AND product_id = $2 AND status = 'reversed'
                        AND metadata->>'reversed_by' = 'tag_removed'",
                )
                .bind(lead_id)
                .bind(product_id)
                .execute(pool)
                .await?;
                if reopened.rows_affected() > 0 {
                    tracing::info!(%lead_id, %product_id,
                        "affiliate commission RE-OPENED to pending — the tag that produces it is back on the lead");
                }
                continue;
            }
            Some(_) => continue,
            None => {}
        }

        // A pending commission used to be written with the amount HARDCODED to 0 and no record of
        // what rate applied, so the row could never be explained or paid. It now carries the rate the
        // one resolver decided, an estimate of what it is worth at the product's list price, and the
        // rule that produced the rate. The authoritative money event remains the conversion webhook,
        // which recomputes from the real sale amount.
        let resolved =
            crate::commission::resolve(pool, Some(product_id), Some(&affiliate_id)).await;
        let list_price: Option<f64> =
            sqlx::query_scalar("SELECT price::float8 FROM affiliate_products WHERE id = $1")
                .bind(product_id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        let estimate = crate::commission::commission_for(list_price.unwrap_or(0.0), resolved.rate);
        let meta = sqlx::types::Json(serde_json::json!({
            "rate": resolved.rate,
            "rate_source": resolved.source,
            "rate_explanation": resolved.explanation,
            "rate_group": resolved.group_name,
            "list_price": list_price,
            "estimate": estimate,
            "note": "pending: computed from the product list price; the conversion webhook recomputes from the real sale amount",
        }));

        sqlx::query(
            "INSERT INTO affiliate_commissions (id, affiliate_id, lead_id, product_id, amount, status, metadata)
             VALUES ($1, $2, $3, $4, $5, 'pending', $6)",
        )
        .bind(Uuid::new_v4())
        .bind(&affiliate_id)
        .bind(lead_id)
        .bind(product_id)
        .bind(estimate)
        .bind(meta)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// RETRACT the tag-based attribution when a product-linked system tag comes OFF a lead.
///
/// kanban t_769b93ba. `attribute_affiliate_on_tags` above is the one writer of the tag-driven
/// commission, and until this card there was no counter-writer: the lead modal's picker can only
/// ADD tags (`apply_lead_tags` merges), so an operator who tagged a lead with the WRONG
/// product-linked system tag left a pending `affiliate_commissions` row behind that nothing in the
/// app could retract — a mis-tag was also a mis-attribution.
///
/// THE DECISION (one arm, chosen by measurement, recorded at the call site): a removed tag REVERSES
/// the row — `status = 'reversed', reversed_at = NOW()` — it does NOT delete it.
///
///   * `src/plan_movement.rs:189` is the app's existing withdrawal mechanism for exactly this money
///     (`status = 'reversed', reversed_at = NOW()` when the customer leaves a paying plan), and
///     `reversed_at` is the column that records WHEN. Deleting the row would erase the audit trail of
///     an attribution that really was made, and would leave nothing for the un-reversal below.
///   * Only a row still `pending` is withdrawn. `pending` means "attributed at tag time, computed
///     from the product list price" (`attribute_affiliate_on_tags` writes it that way; the
///     authoritative money event is the conversion webhook). A row that has already moved on —
///     `earned`/`paid` — is left EXACTLY as it is: withdrawing settled money because a label was
///     taken off a lead is not this arm's business.
///   * The withdrawal is not one-way, and its un-doing is PRECISE. This arm marks the row it
///     withdrew (`metadata->>'reversed_by' = 'tag_removed'`), so re-applying the tag re-opens exactly
///     that row to `pending` (the `Some("reversed")` arm of `attribute_affiliate_on_tags`) — while a
///     row `plan_movement.rs` reversed because the customer left a paying plan keeps no such mark and
///     STAYS reversed, because a tag coming back is not a payment. A real conversion un-reverses that
///     one the only other way there is (`earned` + `reversed_at = NULL`, `plan_movement.rs:172`).
///     Still at most one row per (lead, product).
///
/// `removed_tag_ids` are the tags this request took off the lead; `remaining_tag_ids` are the tags
/// still on it. A product is withdrawn only if NO tag left on the lead still points at it, so a
/// product reachable through a second tag keeps its attribution.
pub async fn reverse_affiliate_on_tags(
    pool: &PgPool,
    lead_id: Uuid,
    removed_tag_ids: &[Uuid],
    remaining_tag_ids: &[Uuid],
) -> std::result::Result<u64, AppError> {
    if removed_tag_ids.is_empty() {
        return Ok(0);
    }

    // Products the tags that came OFF the lead point at (the same predicate, and the same
    // `is_active = true` guard, as the attribution reader above — a retired product is not money).
    let removed_products: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM affiliate_products WHERE system_tag_id = ANY($1) AND is_active = true",
    )
    .bind(removed_tag_ids)
    .fetch_all(pool)
    .await?;
    if removed_products.is_empty() {
        return Ok(0);
    }

    // …minus the ones a tag STILL on the lead points at: the attribution survives if the lead is
    // still reachable to that product through another tag.
    let still_linked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM affiliate_products WHERE system_tag_id = ANY($1) AND is_active = true",
    )
    .bind(remaining_tag_ids)
    .fetch_all(pool)
    .await?;

    let to_reverse: Vec<Uuid> = removed_products
        .into_iter()
        .filter(|p| !still_linked.contains(p))
        .collect();
    if to_reverse.is_empty() {
        return Ok(0);
    }

    // `reversed_by` is what makes the un-reversal precise: `attribute_affiliate_on_tags` re-opens a
    // reversed row ONLY when this arm is the one that reversed it. A row `plan_movement.rs` reversed
    // (the customer left a paying plan) carries no such mark and stays reversed — a tag coming back is
    // not a payment.
    let res = sqlx::query(
        "UPDATE affiliate_commissions
            SET status = 'reversed', reversed_at = NOW(),
                metadata = coalesce(metadata, '{}'::jsonb)
                           || jsonb_build_object('reversed_by', 'tag_removed')
          WHERE lead_id = $1 AND product_id = ANY($2) AND status = 'pending'",
    )
    .bind(lead_id)
    .bind(&to_reverse)
    .execute(pool)
    .await?;

    if res.rows_affected() > 0 {
        tracing::info!(%lead_id, rows = res.rows_affected(),
            "affiliate commission REVERSED — the tag that produced it was removed from the lead");
    }
    Ok(res.rows_affected())
}

/// SET-BASED form of the counter-writer above, for the tag DELETE arm (kanban t_ca4693e3).
///
/// The SAME rule, the same mark and the same restraint as `reverse_affiliate_on_tags` — and it lives
/// here, beside it, so the tag-attribution money rule has one home: only a `pending` row is withdrawn,
/// it is REVERSED (`status='reversed'`, `reversed_at=NOW()`) and marked
/// `metadata->>'reversed_by' = 'tag_removed'` (never deleted, and a row that has already settled —
/// `earned`/`paid` — is left exactly as it is), and a product stays attributed when a tag STILL on the
/// lead points at it.
///
/// The only difference is the shape: deleting a tag strips the name from EVERY lead that carried it in
/// one transaction, so "no remaining tag on this lead points at this product" is evaluated once,
/// set-based, against each lead's OWN stored list — which the caller has already stripped, so what is
/// read here IS the remaining list. That is also why the caller must run this BEFORE deleting the tag
/// row: `affiliate_products.system_tag_id` is `ON DELETE SET NULL`, so after the delete the product
/// link is gone and the attribution could never be resolved (or retracted) again.
pub async fn reverse_affiliate_on_tags_bulk(
    conn: &mut sqlx::PgConnection,
    lead_ids: &[Uuid],
    removed_tag_ids: &[Uuid],
) -> std::result::Result<u64, AppError> {
    if lead_ids.is_empty() || removed_tag_ids.is_empty() {
        return Ok(0);
    }

    // The products the deleted tag(s) pointed at (the same predicate, and the same `is_active`
    // guard, as `attribute_affiliate_on_tags` and the per-lead withdrawal above), minus the ones a
    // tag STILL on the lead points at.
    let res = sqlx::query(
        "UPDATE affiliate_commissions c
            SET status = 'reversed', reversed_at = NOW(),
                metadata = coalesce(c.metadata, '{}'::jsonb)
                           || jsonb_build_object('reversed_by', 'tag_removed')
          WHERE c.status = 'pending'
            AND c.lead_id = ANY($1::uuid[])
            AND c.product_id IN (SELECT id FROM affiliate_products
                                  WHERE system_tag_id = ANY($2::uuid[]) AND is_active = true)
            AND NOT EXISTS (
                SELECT 1
                  FROM leads l
                  JOIN tags t ON (t.tenant_id = l.tenant_id OR t.is_system = true)
                             AND l.tags @> to_jsonb(t.name)
                  JOIN affiliate_products p ON p.system_tag_id = t.id AND p.is_active = true
                 WHERE l.id = c.lead_id AND p.id = c.product_id)",
    )
    .bind(lead_ids)
    .bind(removed_tag_ids)
    .execute(conn)
    .await?;

    if res.rows_affected() > 0 {
        tracing::info!(
            rows = res.rows_affected(),
            leads = lead_ids.len(),
            "affiliate commissions REVERSED — the tag that produced them was DELETED"
        );
    }
    Ok(res.rows_affected())
}
