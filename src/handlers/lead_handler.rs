use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::handlers::workflowswift_push::{deliver_lead, LeadPush, PushOutcome};
use crate::models::lead::*;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LeadQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub stage: Option<String>,
    pub source: Option<String>,
    pub search: Option<String>,
}

#[derive(Serialize)]
pub struct PaginatedLeads {
    pub data: Vec<Lead>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

pub async fn list_leads(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<LeadQuery>,
) -> AppResult<Json<PaginatedLeads>> {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM leads WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;

    let leads = sqlx::query_as::<_, Lead>(
        "SELECT * FROM leads WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(tenant_id)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(PaginatedLeads {
        data: leads,
        total,
        page,
        per_page,
    }))
}

pub async fn create_lead(
    auth: AuthUser,

    State(state): State<AppState>,
    Json(req): Json<CreateLeadRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_limit(&state, tenant_id, "max_leads", "Leads").await?;

    // Check for duplicate email within tenant
    if let Some(ref email) = req.email {
        if !email.trim().is_empty() {
            let existing: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM leads WHERE email = $1 AND tenant_id = $2)",
            )
            .bind(email)
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(false);

            if existing {
                return Err(AppError::BadRequest(format!(
                    "A lead with email '{}' already exists in this workspace",
                    email
                )));
            }
        }
    }

    // `leads.name` is NOT NULL, but a client may legitimately identify a person by parts alone:
    // the mobile capture screen sends `first_name`/`last_name` and has no combined `name` field
    // at all. Binding the absent `name` straight through put NULL into a NOT NULL column, so
    // EVERY lead captured from the app failed with a 500 "Database error". Compose the display
    // name from whatever the caller actually gave us, and answer a genuinely nameless lead with
    // a 400 instead of a database fault.
    let display_name: Option<String> = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let joined = format!(
                "{} {}",
                req.first_name.as_deref().unwrap_or("").trim(),
                req.last_name.as_deref().unwrap_or("").trim()
            )
            .trim()
            .to_string();
            (!joined.is_empty()).then_some(joined)
        });
    let Some(display_name) = display_name else {
        return Err(AppError::BadRequest(
            "A lead needs a name — supply `name`, or at least `first_name`".into(),
        ));
    };

    let lead_id = Uuid::new_v4();
    // The user account this lead flows through — the affiliate attribution anchor.
    let created_by = Uuid::parse_str(&auth.user_id).ok();

    sqlx::query(
        r#"INSERT INTO leads (id, tenant_id, first_name, last_name, name, email, phone, company, source, stage, tags, custom_fields, notes, assigned_to, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"#,
    )
    .bind(lead_id)
    .bind(tenant_id)
    .bind(&req.first_name)
    .bind(&req.last_name)
    .bind(&display_name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.company)
    .bind(&req.source)
    .bind(&req.stage)
    .bind(req.tags.map(|t| serde_json::Value::Array(t.into_iter().map(serde_json::Value::String).collect())))
    .bind(&req.custom_fields)
    .bind(&req.notes)
    .bind(req.assigned_to)
    .bind(created_by)
    .execute(&state.pool)
    .await?;

    // Best-effort push to WorkflowSwift — the REAL POST {WORKFLOWSWIFT_URL}/api/v1/incoming call,
    // the same `deliver_lead` the /api/v1/push/workflowswift leg uses. Until 2026-09-25 this spawn
    // was `push_to_workflowswift`, which only wrote a log line while the route beside it fabricated
    // a success body (kanban t_0a6a93f1); a lead was "pushed" nowhere. Failures are logged and never
    // fail the lead — the same best-effort contract MissedCallRespondr ships for its contacts.
    tokio::spawn({
        let url = state.workflowswift_url.clone();
        let push = LeadPush {
            email: req.email.clone(),
            name: Some(display_name.clone()),
            phone: req.phone.clone(),
            company: req.company.clone(),
            source_entry_id: Some(lead_id.to_string()),
            campaign_slug: None,
            data: json!({
                "lead_id": lead_id,
                "tenant_id": tenant_id,
                "source": req.source,
                "stage": req.stage,
            }),
        };
        async move {
            match deliver_lead(&url, &push).await {
                PushOutcome::Pushed => {
                    tracing::info!("workflowswift: lead {} delivered (workflow matched)", lead_id)
                }
                PushOutcome::NoWorkflow => tracing::info!(
                    "workflowswift: lead {} accepted, no active workflow matches source 'funnelswift' — nothing written there",
                    lead_id
                ),
                PushOutcome::Failed { status, message } => tracing::warn!(
                    "workflowswift: lead {} not delivered (upstream status {:?}): {message}",
                    lead_id,
                    status
                ),
            }
        }
    });

    // Best-effort push to CoreSwift CRM — the ONE CoreSwift path is the tenant BYOK client in
    // `crate::coreswift` (fleet standard 2026-09-20). The previous env/internal-shared-key push
    // was replaced: credentials belong to the tenant, not to a global paste.
    crate::coreswift::spawn_lead_push(
        state.pool.clone(),
        tenant_id,
        crate::coreswift::LeadPayload {
            email: req.email.as_deref().map(str::to_string),
            phone: req.phone.as_deref().map(str::to_string),
            name: Some(display_name.clone()),
            company: req.company.as_deref().map(str::to_string),
            source: req.source.as_deref().map(str::to_string),
            ..Default::default()
        },
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": lead_id, "message": "Lead created"})),
    ))
}

pub async fn get_lead(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Lead>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let lead = sqlx::query_as::<_, Lead>("SELECT * FROM leads WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Lead not found".into()))?;

    Ok(Json(lead))
}

pub async fn update_lead(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLeadRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing =
        sqlx::query_as::<_, Lead>("SELECT * FROM leads WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Lead not found".into()))?;

    // The app edits a person by parts (`first_name`/`last_name`) and never sends a combined
    // `name`. The UPDATE below did not mention first_name/last_name at all and only ever wrote
    // `name`, so renaming a lead from the app was a silent no-op: the old name stayed, and the
    // parts were discarded. Prefer an explicit `name`, else recompose from the parts, else keep
    // what the row already holds.
    let first_name = req.first_name.clone().or(existing.first_name.clone());
    let last_name = req.last_name.clone().or(existing.last_name.clone());
    let name = req
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| {
            let joined = format!(
                "{} {}",
                first_name.as_deref().unwrap_or("").trim(),
                last_name.as_deref().unwrap_or("").trim()
            )
            .trim()
            .to_string();
            (!joined.is_empty()).then_some(joined)
        })
        .or(existing.name.clone())
        .unwrap_or_default();
    let email = req.email.clone().or(existing.email.clone());
    let phone = req.phone.clone().or(existing.phone.clone());
    let company = req.company.clone().or(existing.company.clone());
    let source = req.source.clone().or(existing.source.clone());
    // `Lead.status` decodes as Option since kanban t_d5da34d0 (the column is NULLABLE); the
    // resolved value is unchanged - request, else the stored value, else "active" - and this path
    // still never binds a NULL, so the update can neither create nor clear the NULL state.
    let status = req
        .status
        .clone()
        .or(existing.status.clone())
        .unwrap_or_else(|| "active".to_string());
    let stage = req.stage.clone().or(existing.stage.clone());
    let score = req.score.or(existing.score);
    let notes = req.notes.or(existing.notes);
    let assigned_to = req.assigned_to.or(existing.assigned_to);
    // kanban t_2f2c184b: every other column here resolves request-then-stored, but `tags` was bound
    // from the REQUEST only — so any body that omitted `tags` (the leads Edit modal omits it, and any
    // one-field caller does) NULLed a real, populated column: `leads.tags` is what `assign_lead_tags`
    // and `tag_logic` merge into, and 11 of the 53 live rows carry a value. Keep it when it is not sent.
    let tags = req
        .tags
        .map(|t| serde_json::Value::Array(t.into_iter().map(serde_json::Value::String).collect()))
        .or(existing.tags);
    let custom_fields = req.custom_fields.or(existing.custom_fields);

    sqlx::query(
        r#"UPDATE leads SET name=$1, first_name=$2, last_name=$3, email=$4, phone=$5, company=$6,
           source=$7, status=$8, stage=$9, score=$10, tags=$11, custom_fields=$12, notes=$13,
           assigned_to=$14, updated_at=NOW() WHERE id=$15 AND tenant_id=$16"#,
    )
    .bind(&name)
    .bind(&first_name)
    .bind(&last_name)
    .bind(&email)
    .bind(&phone)
    .bind(&company)
    .bind(&source)
    .bind(&status)
    .bind(&stage)
    .bind(score)
    .bind(&tags)
    .bind(&custom_fields)
    .bind(&notes)
    .bind(assigned_to)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Lead updated"})))
}

pub async fn delete_lead(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let result = sqlx::query("DELETE FROM leads WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Lead not found".into()));
    }

    Ok(Json(json!({"message": "Lead deleted"})))
}

pub async fn assign_lead(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query(
        "UPDATE leads SET assigned_to = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3",
    )
    .bind(req.assigned_to)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Lead assigned"})))
}

pub async fn update_lead_stage(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<StageRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("UPDATE leads SET stage = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
        .bind(&req.stage)
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    // Log activity
    sqlx::query(
        // activity_log.id is uuid NOT NULL with no DEFAULT: the INSERT omitted it, so
        // this route failed 23502 for EVERY caller (not just NULL-email leads) and no
        // stage change was ever recorded. Mint the id like the other tables' rows.
        "INSERT INTO activity_log (id, tenant_id, user_id, action, entity_type, entity_id, metadata) VALUES (gen_random_uuid(), $1, $2, 'stage_change', 'lead', $3, $4)",
    )
    .bind(tenant_id)
    .bind(&auth.user_id)
    .bind(id.to_string())
    .bind(json!({"new_stage": req.stage}))
    .execute(&state.pool)
    .await?;

    // Best-effort push to CoreSwift CRM on stage change — BYOK path (src/coreswift.rs).
    // A stage change only syncs when the tenant has connected CoreSwift.
    //
    // leads.email is NULLABLE; decoding it as a non-Option String failed the whole
    // row and the old `if let Ok(..)` swallowed it, so the push silently never
    // happened for any lead without an email (2 of 53 live rows) — and nothing
    // logged it. Option + an explicit Err arm makes that state visible.
    match sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
        "SELECT name, email, company, phone FROM leads WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some((name, email, company, phone))) => {
            crate::coreswift::spawn_lead_push(
                state.pool.clone(),
                tenant_id,
                crate::coreswift::LeadPayload {
                    email,
                    phone,
                    name: Some(name),
                    company,
                    source: Some("stage_change".to_string()),
                    ..Default::default()
                },
            );
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(
            lead_id = %id,
            error = %e,
            "stage-change lead lookup failed; CoreSwift push skipped"
        ),
    }

    Ok(Json(json!({"message": "Stage updated"})))
}

#[derive(Deserialize)]
pub struct LeadTagsRequest {
    pub tags: Vec<String>,
    pub triggered_by: Option<String>,
}

/// PUT /api/v1/leads/:id/status — kanban t_2f2c184b.
///
/// The leads row badge's "Change Status" control saves here. It called this path before the route
/// existed, so every save was an empty-bodied 404 and the modal was display-only (invisible in the
/// SPA because `api()` ran `r.json()` on the empty body and threw a SyntaxError).
///
/// Deliberately a STATUS-ONLY write. The alternative — pointing the control at `PUT /leads/:id` —
/// rewrites 15 columns from one field and is lossy (`update_lead` bound `tags` from the request
/// only, so a body without `tags` NULLed a real column). One control, one column.
///
/// The value VOCABULARY is not enforced here: which space is canonical for a lead status
/// (`leads.status` lowercase vs the tenant's Title-case `tenant_settings.lead_stages`) is decided
/// by kanban t_adf187f9, and if a value-space check belongs anywhere it belongs in one place for
/// both leads controls. What IS enforced is what the column physically is: `VARCHAR(50)`, so a
/// blank or over-long value gets a 400 naming the limit instead of a 500 from the database.
pub async fn update_lead_status(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<LeadStatusRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let status = req.status.trim().to_string();
    if status.is_empty() {
        return Err(AppError::BadRequest("status is required".into()));
    }
    if status.chars().count() > 50 {
        return Err(AppError::BadRequest(
            "status must be 50 characters or fewer".into(),
        ));
    }

    let res = sqlx::query(
        "UPDATE leads SET status = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3",
    )
    .bind(&status)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("Lead not found".into()));
    }

    Ok(Json(
        json!({"message": "Lead status updated", "status": status}),
    ))
}

pub async fn assign_lead_tags(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<LeadTagsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Get current lead tags.
    // `leads.tags` is NULLABLE and 42 of the 53 live rows carry NULL (measured 2026-09-25,
    // kanban t_ae84b186): decoding it straight into `serde_json::Value` makes sqlx answer
    // "unexpected null; try decoding as an `Option`" and this route 500ed for 79% of leads —
    // which is also the route that fires the cross-app tag sync to CoreSwift. COALESCE keeps a
    // tagless lead readable as an empty tag list (tag_logic.rs already reads the same column as
    // Option<Value> for the same reason).
    let row = sqlx::query_as::<_, (serde_json::Value,)>(
        "SELECT COALESCE(tags, '[]'::jsonb) FROM leads WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let row = row.ok_or_else(|| AppError::NotFound("Lead not found".into()))?;
    let (tag_val,) = row;
    let mut current_tags: Vec<String> = match tag_val {
        serde_json::Value::Array(ref arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    };

    // Determine which tags are new (to evaluate rules against)
    let new_tags: Vec<Uuid> = {
        let all_tags =
            sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM tags WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_all(&state.pool)
                .await?;
        all_tags
            .into_iter()
            .filter(|(_, name)| req.tags.contains(name) && !current_tags.contains(name))
            .map(|(id, _)| id)
            .collect()
    };

    // Merge new tags
    for t in &req.tags {
        if !current_tags.contains(t) {
            current_tags.push(t.clone());
        }
    }

    // Evaluate tag rules
    let (to_remove, to_add) = crate::tag_logic::evaluate_tag_rules(
        &state.pool,
        tenant_id,
        &current_tags
            .iter()
            .map(|s| serde_json::Value::String(s.clone()))
            .collect::<Vec<_>>(),
        &new_tags,
    )
    .await?;

    // Apply rule results
    current_tags.retain(|t| !to_remove.contains(t));
    for t in &to_add {
        if !current_tags.contains(t) {
            current_tags.push(t.clone());
        }
    }

    let tags_json: serde_json::Value = serde_json::Value::Array(
        current_tags
            .iter()
            .map(|t| serde_json::Value::String(t.clone()))
            .collect(),
    );

    sqlx::query("UPDATE leads SET tags = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
        .bind(&tags_json)
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    // Log change
    let triggered_by = req.triggered_by.unwrap_or_else(|| "manual".to_string());
    crate::tag_logic::log_tag_change(
        &state.pool,
        tenant_id,
        id,
        &req.tags,
        &to_remove,
        &triggered_by,
    )
    .await?;

    // Attribute affiliate commissions for any product-linked system tags just applied.
    // Combines directly-assigned new tags + rule-driven added tags, resolved to tag IDs.
    let mut all_new_tag_ids = new_tags.clone();
    if !to_add.is_empty() {
        let rule_tag_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM tags WHERE name = ANY($1) AND (tenant_id = $2 OR is_system = true)",
        )
        .bind(&to_add)
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?;
        all_new_tag_ids.extend(rule_tag_ids);
    }
    crate::tag_logic::attribute_affiliate_on_tags(&state.pool, id, &all_new_tag_ids).await?;

    // Fire cross-app sync to CoreSwift CRM
    if !state.coreswift_url.is_empty() {
        // leads.company is NULLABLE and holds NULL for 43 of the 53 live rows;
        // leads.email likewise for 2. The hub's TagSyncLead wants a required String
        // email and an Option<String> company
        // (CoreSwift-CRM src/webhooks/cross_app_tag_sync.rs:32), so email is
        // COALESCEd to '' (a JSON null would be refused with a 422) while company
        // decodes as Option and travels as null.
        let lead = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT name, COALESCE(email, '') AS email, company FROM leads WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;

        if let Some((lname, lemail, lcompany)) = lead {
            let sync_payload = serde_json::json!({
                "event": "tag_sync",
                "source_app": "funnelswift",
                "tenant_id": tenant_id,
                "lead": {
                    "id": id,
                    "name": lname,
                    "email": lemail,
                    "company": lcompany
                },
                "tags": current_tags,
                "added_tags": req.tags,
                "removed_tags": to_remove,
                "triggered_by": triggered_by
            });

            // Fire and forget — don't block the response
            let url = format!("{}/api/v1/webhooks/cross-app/tag-sync", state.coreswift_url);
            let internal_sync_key = state.internal_sync_key.clone();
            let client = reqwest::Client::new();
            tokio::spawn(async move {
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

    Ok(Json(json!({
        "message": "Tags updated",
        "tags": current_tags,
        "rules_applied": to_remove.len() + to_add.len()
    })))
}

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
}

pub async fn export_leads(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_flag(&state, tenant_id, "has_import_export", "Lead export").await?;

    let leads = sqlx::query_as::<_, Lead>(
        "SELECT * FROM leads WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let csv_leads: Vec<serde_json::Value> = leads
        .iter()
        .map(|l| {
            json!({
                "id": l.id,
                "name": l.name,
                "email": l.email,
                "phone": l.phone,
                "company": l.company,
                "source": l.source,
                "stage": l.stage,
                "status": l.status,
                "score": l.score,
                "notes": l.notes,
                "created_at": l.created_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "format": query.format.as_deref().unwrap_or("json"),
        "count": leads.len(),
        "data": csv_leads
    })))
}
