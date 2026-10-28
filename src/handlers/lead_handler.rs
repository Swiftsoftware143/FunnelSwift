use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::handlers::workflowswift_push::{deliver_lead, LeadPush, PushOutcome};
use crate::models::lead::*;
use crate::security::lead_fields;
use crate::state::AppState;

/// Refuse a lead field the column cannot store, naming it, instead of letting the INSERT/UPDATE
/// fail. Measured live 2026-10-17: `POST /api/v1/leads` with a 300-character `name` answered
/// `500 {"error":"Database error"}`, and an 80-character `phone` did the same. The caps are the
/// live column widths (see `security::lead_fields`).
fn check_storable(field: &str, value: Option<&str>, cap: usize) -> AppResult<()> {
    lead_fields::check(field, value, cap).map_err(AppError::UnprocessableEntity)
}

#[derive(Deserialize)]
pub struct LeadQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub stage: Option<String>,
    pub source: Option<String>,
    pub search: Option<String>,
    // WHICH FUNNEL PRODUCED THIS LEAD (cron 2026-10-24). `submit_funnel_lead` stamps the funnel id
    // into `custom_fields.funnel_id`, and `list_funnels` counts on exactly that key — but nothing
    // could ASK for those rows: the Funnels screen's "Leads" number had to approximate the answer by
    // typing the funnel's NAME into the search box, which matches any lead whose text mentions it
    // (two funnels called "Spring Sale" and "Spring Sale 2" each showed the other's people) and only
    // ever searched the single page the console had already loaded. This is the filter that makes
    // the count answerable, and it is the same key the count is derived from, so the number and the
    // list behind it cannot drift.
    pub funnel_id: Option<String>,
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

    // THE FILTERS THE QUERY ALREADY DECLARED, now actually applied (2026-10-18). `LeadQuery` has
    // carried `search`, `stage` and `source` since it was written, and this handler bound NONE of
    // them: every call returned the whole workspace, so the console's search box only ever filtered
    // the rows it had already loaded (and the API answered `total` for the unfiltered set). Measured
    // live before the fix: `GET /leads?search=Lead 061` answered the same 120 rows with
    // `total=120`.
    //
    // The ORDER BY also gains a tiebreak. Every lead a single import writes shares one
    // `created_at` (one transaction, one `now()`), and with a non-unique sort key PostgreSQL is free
    // to return them in a different order per query — i.e. LIMIT/OFFSET paging over an import could
    // repeat a row on page 2 and drop another entirely.
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{s}%"));
    let stage = query
        .stage
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let source = query
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // The funnel attribution key is the same expression `list_funnels` counts:
    // `custom_fields->>'funnel_id'`, compared as text so a caller cannot smuggle a non-uuid.
    let funnel_id = query
        .funnel_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leads
         WHERE tenant_id = $1
           AND ($2::text IS NULL
                OR name ILIKE $2 OR email ILIKE $2 OR company ILIKE $2
                OR phone ILIKE $2 OR notes ILIKE $2)
           AND ($3::text IS NULL OR stage = $3)
           AND ($4::text IS NULL OR source = $4)
           AND ($5::text IS NULL OR custom_fields->>'funnel_id' = $5)",
    )
    .bind(tenant_id)
    .bind(&search)
    .bind(&stage)
    .bind(&source)
    .bind(&funnel_id)
    .fetch_one(&state.pool)
    .await?;

    let leads = sqlx::query_as::<_, Lead>(
        "SELECT * FROM leads
         WHERE tenant_id = $1
           AND ($2::text IS NULL
                OR name ILIKE $2 OR email ILIKE $2 OR company ILIKE $2
                OR phone ILIKE $2 OR notes ILIKE $2)
           AND ($3::text IS NULL OR stage = $3)
           AND ($4::text IS NULL OR source = $4)
           AND ($5::text IS NULL OR custom_fields->>'funnel_id' = $5)
         ORDER BY created_at DESC, id DESC
         LIMIT $6 OFFSET $7",
    )
    .bind(tenant_id)
    .bind(&search)
    .bind(&stage)
    .bind(&source)
    .bind(&funnel_id)
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
    // Before ANY write: a value the column cannot hold is refused with the field named.
    check_storable("name", req.name.as_deref(), lead_fields::TEXT_CHARS)?;
    check_storable(
        "first_name",
        req.first_name.as_deref(),
        lead_fields::TEXT_CHARS,
    )?;
    check_storable(
        "last_name",
        req.last_name.as_deref(),
        lead_fields::TEXT_CHARS,
    )?;
    check_storable("email", req.email.as_deref(), lead_fields::TEXT_CHARS)?;
    check_storable("phone", req.phone.as_deref(), lead_fields::PHONE_CHARS)?;
    check_storable("company", req.company.as_deref(), lead_fields::TEXT_CHARS)?;
    check_storable("source", req.source.as_deref(), lead_fields::SOURCE_CHARS)?;
    check_storable("stage", req.stage.as_deref(), lead_fields::STAGE_CHARS)?;
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
    // The composed name is what lands in `leads.name`; two individually-storable parts can still
    // add up to more than the column holds, which used to be a 500 from the INSERT.
    if display_name.chars().count() > lead_fields::TEXT_CHARS {
        return Err(AppError::UnprocessableEntity(
            "name: first_name and last_name together are longer than 255 characters".into(),
        ));
    }

    let lead_id = Uuid::new_v4();
    // The user account this lead flows through — the affiliate attribution anchor.
    let created_by = Uuid::parse_str(&auth.user_id).ok();

    sqlx::query(
        r#"INSERT INTO leads (id, tenant_id, first_name, last_name, name, email, phone, company, source, stage, tags, custom_fields, notes, assigned_to, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, COALESCE(NULLIF(TRIM($10), ''), 'New'), $11, $12, $13, $14, $15)"#,
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

    // kanban t_d4ce013c: the served lead form's tag picker posts the picked tags BY ID (`tag_ids`),
    // and this route bound no such field — nothing denies unknown fields, so serde dropped it and an
    // operator could pick a system tag, watch the save answer 201, and store nothing. Resolve the ids
    // with the SAME predicate `POST /api/v1/leads/:id/tags` uses (system tags are offered to every
    // workspace by GET /api/v1/tags, so they must resolve here too — the 669a2a7 fix) and run them
    // through `apply_lead_tags`, the ONE writer of `leads.tags`. ARM (a) of the card, chosen by
    // measurement: arm (b) would need this create to return first and a second POST to
    // /api/v1/leads/:id/tags afterwards, i.e. a second round trip AND a lead that is already saved
    // with no tags if that call fails.
    let created_tag_names = resolve_tag_ids(&state.pool, tenant_id, req.tag_ids.as_deref()).await?;
    if !created_tag_names.is_empty() {
        apply_lead_tags(
            &state,
            tenant_id,
            lead_id,
            &created_tag_names,
            &[],
            "lead_create",
        )
        .await?;
    }

    // Outbound webhook delivery (kanban t_431faa99): the console's own Create Lead is a writer of a
    // `leads` row like any capture path, so it emits the same `lead.created`. Emitted AFTER the tag
    // block so the payload carries the tags that were actually stored.
    crate::webhooks::spawn(
        state.pool.clone(),
        tenant_id,
        crate::webhooks::LEAD_CREATED,
        crate::webhooks::lead_created(
            lead_id,
            Some(display_name.as_str()),
            req.email.as_deref(),
            req.phone.as_deref(),
            req.company.as_deref(),
            req.source.as_deref(),
            &created_tag_names,
        ),
    );

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

    // The tag→free-account outcome, in the answer the console's Save already reads. The tag write
    // answers 201 whether the sibling minted a workspace or refused the address, so without this
    // the person who clicked the tag saw "Lead created" and could not tell which had happened.
    let provisioning = provisioning_sentences(&state.pool, tenant_id, lead_id)
        .await
        .unwrap_or_default();

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": lead_id, "message": "Lead created", "provisioning": provisioning})),
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

/// What the tag-driven free-account flow did for THIS lead.
///
/// `POST /api/v1/leads/:id/tags` runs the mapping in `src/app_provision.rs` AFTER the tag write
/// (fire-and-forget, so a failed sibling call can never fail the tag) and records one
/// `provisioning_log` row per attempt. The only door onto those rows was
/// `GET /api/v1/admin/provisioning-log` — ADMIN-ONLY and tenant-wide — so the person who actually
/// clicks the tag, who is usually not an admin, was told "Tags updated" and nothing else: no way to
/// learn that a workspace had just been minted in another app, or that the target refused the
/// address. Measured 2026-10-28: the tag answered `200 Tags updated` while WorkflowSwift minted an
/// account (status `provisioned`, http 201) — visible only by reading SQL afterwards.
///
/// Scoped to the caller's OWN tenant AND to a lead in it, so another workspace's lead id answers
/// 404 rather than its history. Newest first; `count: 0` is the honest answer for a lead whose tags
/// carry no mapping, never an error.
/// The operator-readable name of a provisioner target (`provisioning_log.source_app`).
fn provision_app_label(slug: &str) -> &str {
    match slug {
        "workflowswift" => "WorkflowSwift",
        "incentiveswift" => "IncentiveSwift",
        "adaswift" => "ADASwift",
        "missedcallrespondr" => "MissedCall",
        "funnelswift" => "FunnelSwift",
        "coreswiftcrm" | "coreswift-crm" | "coreswift" => "CoreSwift CRM",
        other => other,
    }
}

/// One plain-English sentence per provisioning attempt for this lead, newest first.
///
/// The tag write answers `200`/`201` whether the sibling app minted a workspace, refused the
/// address or could not be reached — every outcome is a `provisioning_log` row and nothing else —
/// so a caller that reads only the status code is told "saved" for all three. This is the sentence
/// the console shows and the lead-write response carries (`provisioning`).
///
/// Tenant-scoped on `provisioning_log.tenant_id` as well as `lead_id`: another workspace's attempt
/// must never be readable through a lead id. A read failure answers an empty list instead of
/// failing the lead write — a lead that saved must not be reported as failed because its log is
/// unreadable.
async fn provisioning_sentences(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    lead_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT source_app, plan_slug, status, error_message FROM provisioning_log \
          WHERE lead_id = $1 AND tenant_id = $2 ORDER BY created_at DESC",
    )
    .bind(lead_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(source_app, plan_slug, status, error_message)| {
            let app = provision_app_label(&source_app);
            let plan_owned = plan_slug.unwrap_or_default();
            let plan = plan_owned.trim();
            let why_owned = error_message.unwrap_or_default();
            let why = why_owned.trim();
            match status.as_str() {
                "provisioned" => {
                    if plan.is_empty() {
                        format!("A free {app} account was created for this lead.")
                    } else {
                        format!("A free {app} account was created for this lead (plan: {plan}).")
                    }
                }
                "already_exists" => format!(
                    "{app} already had an account for this address, so nothing new was created."
                ),
                "refused" => {
                    if why.is_empty() {
                        format!("{app} refused to create the free account.")
                    } else {
                        format!("{app} refused to create the free account: {why}.")
                    }
                }
                "invalid" => {
                    if why.is_empty() {
                        format!("{app} rejected this lead's address as invalid.")
                    } else {
                        format!("{app} rejected this lead's address as invalid: {why}.")
                    }
                }
                _ => {
                    if why.is_empty() {
                        format!("{app} could not be reached, so no account was created.")
                    } else {
                        format!("{app} could not create the account: {why}.")
                    }
                }
            }
        })
        .collect())
}

pub async fn lead_provisioning_history(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Own the lead BEFORE reading its trail: an id from another workspace must answer 404, never
    // another tenant's provisioning history.
    let owned: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM leads WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    if owned.is_none() {
        return Err(AppError::NotFound("Lead not found".into()));
    }

    let rows: Vec<(
        Uuid,
        String,
        Option<String>,
        String,
        Option<i32>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, source_app, plan_slug, status, http_status, error_message, created_at \
         FROM provisioning_log WHERE lead_id = $1 AND tenant_id = $2 ORDER BY created_at DESC",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let attempts: Vec<serde_json::Value> = rows
        .iter()
        .map(
            |(id, source_app, plan_slug, status, http_status, error_message, created_at)| {
                json!({
                    "id": id.to_string(),
                    "source_app": source_app,
                    "plan_slug": plan_slug,
                    "status": status,
                    "http_status": http_status,
                    "error_message": error_message,
                    "created_at": created_at.to_rfc3339(),
                })
            },
        )
        .collect();

    let sentences = provisioning_sentences(&state.pool, tenant_id, id)
        .await
        .unwrap_or_default();

    Ok(Json(json!({
        "count": attempts.len(),
        "attempts": attempts,
        "sentences": sentences,
    })))
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

    // The update writes only the fields it was sent, so check exactly those (the stored value it
    // falls back to already fits, or it would not be stored).
    check_storable("name", req.name.as_deref(), lead_fields::TEXT_CHARS)?;
    check_storable(
        "first_name",
        req.first_name.as_deref(),
        lead_fields::TEXT_CHARS,
    )?;
    check_storable(
        "last_name",
        req.last_name.as_deref(),
        lead_fields::TEXT_CHARS,
    )?;
    check_storable("email", req.email.as_deref(), lead_fields::TEXT_CHARS)?;
    check_storable("phone", req.phone.as_deref(), lead_fields::PHONE_CHARS)?;
    check_storable("company", req.company.as_deref(), lead_fields::TEXT_CHARS)?;
    check_storable("source", req.source.as_deref(), lead_fields::SOURCE_CHARS)?;
    check_storable("stage", req.stage.as_deref(), lead_fields::STAGE_CHARS)?;
    check_storable("status", req.status.as_deref(), lead_fields::STATUS_CHARS)?;

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

    // kanban t_d4ce013c: the same `tag_ids` hole as `create_lead`, on the served modal's Save
    // (PUT /api/v1/leads/:id). The UPDATE above rebinds the STORED `tags` value — the served modal
    // sends no `tags` key, and the `.or(existing.tags)` guard from t_2f2c184b keeps a populated
    // column intact for every caller that omits it — so `apply_lead_tags` below is the single
    // semantic writer of a new tag list on this route, and it is the very function
    // `POST /api/v1/leads/:id/tags` is made of (one implementation, one rule evaluation, one
    // `attribute_affiliate_on_tags` call, one cross-app sync per request).
    let updated_tag_names = resolve_tag_ids(&state.pool, tenant_id, req.tag_ids.as_deref()).await?;
    if !updated_tag_names.is_empty() {
        apply_lead_tags(
            &state,
            tenant_id,
            id,
            &updated_tag_names,
            &[],
            "lead_update",
        )
        .await?;
    }

    // Same contract as `create_lead`: the lead write's answer carries what the tag-driven
    // free-account flow did, so the modal can tell the operator instead of closing on "saved".
    let provisioning = provisioning_sentences(&state.pool, tenant_id, id)
        .await
        .unwrap_or_default();

    Ok(Json(
        json!({"message": "Lead updated", "lead_id": id, "provisioning": provisioning}),
    ))
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

    // The owner must be a person IN THIS WORKSPACE. The route used to take any uuid it was handed
    // and write it straight into `leads.assigned_to` — nothing checked the row belonged to the
    // caller's tenant, so a workspace could hand its lead to another workspace's user id (a row
    // that would then render as an owner nobody in this tenant can name). Checked before the
    // UPDATE, so a refusal leaves the lead exactly as it was.
    if let Some(owner) = req.assigned_to {
        let in_tenant: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND tenant_id = $2")
                .bind(owner)
                .bind(tenant_id)
                .fetch_optional(&state.pool)
                .await?;
        if in_tenant.is_none() {
            return Err(AppError::BadRequest(
                "That person is not in this workspace".into(),
            ));
        }
    }

    let result = sqlx::query(
        "UPDATE leads SET assigned_to = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3",
    )
    .bind(req.assigned_to)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Lead not found".into()));
    }

    Ok(Json(json!({
        "message": if req.assigned_to.is_some() { "Lead assigned" } else { "Lead unassigned" },
        "assigned_to": req.assigned_to,
    })))
}

/// `GET /api/v1/team/members` — the people in the CALLER's workspace, for the lead owner picker.
///
/// Why it exists: `POST /leads/:id/assign` has been in the router since the leads table did, and
/// NOTHING in the console could reach it — the operator had no way to give a lead to a teammate,
/// so `assigned_to` was a column only an API client could write. The picker needs a list of
/// people, and the only list route was `/api/v1/admin/tenants/:id/users`, which is platform-admin
/// (correctly): a workspace must be able to see its OWN people without being an install operator.
///
/// Scoped from the CREDENTIAL, never from the request: the tenant comes from the token, so the
/// route cannot be pointed at somebody else's workspace, and it returns no password material.
pub async fn team_members(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // Active people only: an owner picker that offers a disabled login hands leads to somebody who
    // cannot sign in to work them.
    let rows = sqlx::query(
        "SELECT id, name, email, role::text AS role
           FROM users
          WHERE tenant_id = $1 AND is_active = true
          ORDER BY lower(coalesce(nullif(name, ''), email)), email",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let members: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let name: String = r.try_get("name").unwrap_or_default();
            let email: String = r.try_get("email").unwrap_or_default();
            let label = if name.trim().is_empty() {
                email.clone()
            } else {
                name
            };
            json!({
                "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
                "name": label,
                "email": email,
                "role": r.try_get::<String, _>("role").unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(json!({ "members": members })))
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

    let result = sqlx::query(
        "UPDATE leads SET stage = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3",
    )
    .bind(&req.stage)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Lead not found".into()));
    }

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
    /// Tag NAMES to put ON the lead (merged into the stored list; nothing is removed).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Tag NAMES to take OFF the lead — the removal arm (kanban t_769b93ba).
    ///
    /// This is the one thing the app never had: the picker could only ADD, so an operator who tagged
    /// a lead with the wrong system tag could not undo it in-app, and a product-linked system tag
    /// writes a pending `affiliate_commissions` row (`attribute_affiliate_on_tags`) — a mis-tag was
    /// also a mis-attribution nothing could retract. Names are matched exactly against the lead's
    /// OWN stored list (that is how `leads.tags` stores tags) and scoped to the caller's tenant;
    /// a name that is not on the lead is a no-op. An explicit removal WINS over a re-add of the same
    /// name in one request (see `apply_lead_tags`), and a tag RULE re-adding it still wins over the
    /// removal — the rule engine runs after this arm.
    #[serde(default)]
    pub remove: Vec<String>,
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
/// The value VOCABULARY is deliberately NOT enforced here, and kanban t_adf187f9 decided why:
/// `leads.status` is its OWN five-value lowercase lifecycle — new, contacted, qualified, converted,
/// lost — and it is NOT the tenant's configured pipeline. The pipeline is the OTHER column,
/// `leads.stage` (VARCHAR(100) DEFAULT 'New'): it is fed by `tenant_settings.lead_stages`, the list
/// the "Lead Stages" screen edits, which `GET /api/v1/dashboard/stats` groups `leads_by_stage` by
/// and which `POST /leads/:id/stage` / the affiliate `to_stage` webhook write. The five status
/// values are therefore declared in exactly ONE place — the served shell's `LD_STATUS_DEFAULT`
/// (`www-app/index.html`, read through `ldStatuses()` by both leads controls) — and a second copy
/// here would be the same three-way drift in a new place. Out-of-list values are accepted on
/// purpose: the shell appends a row's own value to the control, so a no-touch save cannot rewrite
/// it. What IS enforced is what the column physically is: `VARCHAR(50)`, so a blank or over-long
/// value gets a 400 naming the limit instead of a 500 from the database.
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

/// Resolve the tag IDs the served lead form's picker posts (`tag_ids`) to tag NAMES.
///
/// kanban t_d4ce013c. The predicate is the SAME one the name path below uses and the same one
/// GET /api/v1/tags lists with (`tag_handler.rs:22`, `tenant_id = $1 OR is_system = true`): the
/// picker offers the SYSTEM tag vocabulary (owned by the System tenant), so a tenant-only lookup
/// would resolve every system tag in the picker to nothing — the exact hole 669a2a7 closed on the
/// name path, and the reason a restored-and-linked system tag must resolve here too.
async fn resolve_tag_ids(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    ids: Option<&[Uuid]>,
) -> AppResult<Vec<String>> {
    let Some(ids) = ids.filter(|v| !v.is_empty()) else {
        return Ok(vec![]);
    };
    let names = sqlx::query_scalar::<_, String>(
        "SELECT name FROM tags WHERE id = ANY($1) AND (tenant_id = $2 OR is_system = true)",
    )
    .bind(ids)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(names)
}

/// THE one implementation of "make this lead's tag list be this" — the ONE writer of `leads.tags`.
///
/// kanban t_d4ce013c extracted it out of `assign_lead_tags` so the lead write path
/// (POST /api/v1/leads, PUT /api/v1/leads/:id) could run the picker's `tag_ids` through the SAME code
/// instead of a second implementation. kanban t_769b93ba added the other direction: `req_remove`
/// takes names OFF the lead, which the merge below could never do. One implementation of: merge
/// `req_tags` into the stored list, take `req_remove` off it, evaluate the tag rules, write
/// `leads.tags` ONCE, write one `tag_change_log` row, run `attribute_affiliate_on_tags` and its
/// counter-writer `reverse_affiliate_on_tags` (the tag-based affiliate money path t_c06d643c proved)
/// and fire the CoreSwift cross-app sync. `POST /api/v1/leads/:id/tags` (`assign_lead_tags` below) is
/// a thin wrapper over this, so its behaviour is unchanged for a caller that sends only `tags`.
///
/// Returns the lead's final tag list and how many rule actions fired.
async fn apply_lead_tags(
    state: &AppState,
    tenant_id: Uuid,
    id: Uuid,
    req_tags: &[String],
    req_remove: &[String],
    triggered_by: &str,
) -> AppResult<(Vec<String>, usize)> {
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
    // The lead's tag list as it was BEFORE this request. The provisioning arm below needs the
    // NEWLY ADDED names only (kanban t_847f9d63): a tag already on the lead is not an event, and a
    // tag taken OFF the lead must never delete an account.
    let original_tags = current_tags.clone();

    // Determine which tags are new (to evaluate rules against).
    //
    // `is_system = true` is part of the predicate, and that is the whole point (kanban t_c06d643c,
    // measured live 2026-09-26): the system tag vocabulary lives under the System tenant
    // (00000000-0000-0000-0000-000000000001), and GET /api/v1/tags — the list this app's own tag
    // picker loads (tag_handler.rs:22 `WHERE tenant_id = $1 OR is_system = true`, and the served
    // www-app/index.html `tg-select` that collects the picked ids) — OFFERS those tags. So a caller
    // can post a system tag name here. With a tenant-only lookup the name resolved to no id, this
    // route wrote the name into leads.tags and then handed `attribute_affiliate_on_tags` an EMPTY
    // id list, so tag-based attribution credited nothing at all — measured on the deployed binary
    // 5376bb3a: POST with `["FunnelSwift — Capture Free"]` returned 200 with the tag stored and
    // zero affiliate_commissions rows, while the product link it was supposed to credit resolved
    // (one active product behind that tag). The rule-driven arm below (line ~630) already resolves
    // names with `OR is_system = true`; this is the same list for a directly-assigned tag.
    let new_tags: Vec<Uuid> = {
        let all_tags = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, name FROM tags WHERE tenant_id = $1 OR is_system = true",
        )
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?;
        all_tags
            .into_iter()
            .filter(|(_, name)| req_tags.contains(name) && !current_tags.contains(name))
            .map(|(id, _)| id)
            .collect()
    };

    // Merge new tags
    for t in req_tags {
        if !current_tags.contains(t) {
            current_tags.push(t.clone());
        }
    }

    // ── REMOVE arm (kanban t_769b93ba) ──────────────────────────────────────────────────────────
    // `req_remove` takes tag NAMES off the lead. Until this card the ONLY thing this function could
    // do was merge, so an operator who tagged a lead with the wrong system tag could not undo it
    // in-app — and a product-linked system tag writes a pending `affiliate_commissions` row, so the
    // mis-tag was also a mis-attribution nothing could retract (see `reverse_affiliate_on_tags`,
    // called below).
    //
    // Applied AFTER the merge on purpose: an explicit removal WINS over a re-add of the same name in
    // one request, because `remove` is the destructive, explicit retraction and letting the merge
    // resurrect it would keep the very row this arm exists to retract. Matched by exact name against
    // the lead's OWN stored list (that is how `leads.tags` stores tags), tenant-scoped through the
    // SELECT and the UPDATE above; a name that is not on the lead is a no-op and is not logged.
    // A tag RULE that re-adds the removed name still wins — the rule engine runs AFTER this arm.
    let mut explicitly_removed: Vec<String> = vec![];
    for t in req_remove {
        if current_tags.contains(t) {
            current_tags.retain(|c| c != t);
            explicitly_removed.push(t.clone());
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

    // Every name this request took OFF the lead: the explicit removals plus the rule-driven ones.
    // The rule engine's removals count because their premise is the same — the tag is no longer on
    // the lead, so the attribution it produced is no longer true.
    let mut removed_tags: Vec<String> = explicitly_removed;
    for t in &to_remove {
        if !removed_tags.contains(t) {
            removed_tags.push(t.clone());
        }
    }

    // Log change
    crate::tag_logic::log_tag_change(
        &state.pool,
        tenant_id,
        id,
        req_tags,
        &removed_tags,
        triggered_by,
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

    // ── RETRACT the attribution for a product-linked tag this request took OFF the lead ─────────
    // (kanban t_769b93ba). THE DECISION: REVERSE the pending row — `status='reversed',
    // reversed_at=NOW()` — never delete it, and never touch a row that has already settled. It is
    // the app's own withdrawal mechanism for this money (`src/plan_movement.rs:189`), it keeps the
    // audit trail of an attribution that really was made, and it is undone the same way the app
    // already undoes it: re-applying the tag re-opens the same row to pending
    // (`tag_logic::attribute_affiliate_on_tags`, which only un-does a row this arm reversed — it
    // marks the row `reversed_by='tag_removed'`, so a row `plan_movement` reversed because the
    // customer left a paying plan stays reversed), and a real conversion settles it the other way
    // (`earned` + `reversed_at = NULL`, `plan_movement.rs:172`). Still at most one row per
    // (lead, product).
    if !removed_tags.is_empty() {
        let removed_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM tags WHERE name = ANY($1) AND (tenant_id = $2 OR is_system = true)",
        )
        .bind(&removed_tags)
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?;
        // A tag left on the lead that points at the SAME product keeps the attribution: only the
        // products no remaining tag points at are withdrawn.
        let remaining_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM tags WHERE name = ANY($1) AND (tenant_id = $2 OR is_system = true)",
        )
        .bind(&current_tags)
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?;
        crate::tag_logic::reverse_affiliate_on_tags(&state.pool, id, &removed_ids, &remaining_ids)
            .await?;
    }

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
                "added_tags": req_tags,
                "removed_tags": removed_tags,
                "triggered_by": triggered_by
            });

            // Fire and forget — don't block the response
            let url = format!("{}/api/v1/webhooks/cross-app/tag-sync", state.coreswift_url);
            let internal_sync_key = state.internal_sync_key.clone();
            let client = reqwest::Client::new();
            // HARNESS RESIDUE STAYS OUT OF THE HUB. This arm is the one that actually wrote probe
            // people into CoreSwift-CRM's contact list (measured 2026-10-20: tagging a
            // `probe-...@example.invalid` lead created a contact there within 22 ms, and it
            // outlived the lead's deletion — 360 such contacts had accumulated). A reserved test
            // address can never belong to a customer, so the sync is skipped and said out loud
            // instead of quietly filling the operator's CRM with fake people.
            if crate::coreswift::is_reserved_test_email(&lemail) {
                tracing::info!(
                    lead = %id,
                    "[coreswift] cross-app tag sync skipped: {} is a reserved test address",
                    lemail
                );
            } else {
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
    }

    // ── APP FREE-ACCOUNT PROVISIONING (kanban t_847f9d63) ───────────────────────────────────────
    // THE tag → free account executor. David, 2026-10-05: tagging a lead with a software must
    // create a FREE ACCOUNT in that app, upgradeable from inside that app. The mapping already
    // existed on the seven `<App> — Free` system tags (`tags.source_app` + `tags.plan_slug`, plus
    // the new `tags.provisions_account`); `src/app_provision.rs` is the one client that reads it.
    //
    // What is passed in: only the names this request ADDED to the lead — directly assigned
    // (`req_tags`) and rule-driven (`to_add`) alike, computed as "present now, absent before".
    // That is deliberately the inverse of `removed_tags` and it is what makes the two contracts
    // hold: a tag already on the lead is not a new event (re-applying answers `already_exists` at
    // the target, never a second account), and a tag taken OFF the lead performs NO account action
    // — an account is never deleted because a tag came off a lead (design §3.2).
    //
    // `provisions_account = false`, a NULL mapping, and `source_app = 'funnelswift'` are all
    // filtered INSIDE the provisioner, so this call site stays a single honest sentence: "these
    // tags are new; execute whatever mapping they carry". A failure inside can never fail the tag
    // write — every attempt is recorded in `provisioning_log` instead (see that module).
    let added_tags: Vec<String> = current_tags
        .iter()
        .filter(|t| !original_tags.contains(t))
        .cloned()
        .collect();
    if !added_tags.is_empty() {
        crate::app_provision::provision_for_added_tags(state, tenant_id, id, &added_tags).await;
    }

    Ok((current_tags, to_remove.len() + to_add.len()))
}

/// POST /api/v1/leads/:id/tags — the API surface for assigning tags to a lead.
///
/// kanban t_d4ce013c: this route had NO caller in any served shell (the served lead form posted
/// `tag_ids` on the lead write path instead, which dropped them), so the flow t_c06d643c fixed and
/// proved was reachable only by an API client. The route is a thin wrapper over `apply_lead_tags`,
/// which the lead create/update path now shares. The served picker keeps using the lead write path
/// for the ADD half (arm (a) of t_d4ce013c, chosen by measurement): the create cannot call this
/// route at all without a second round trip, because the lead id it needs only exists after the
/// create returns.
///
/// kanban t_769b93ba: this route is where the REMOVE half lands, because a stored tag can only be
/// removed from a lead that already exists. `{"remove":[<name>]}` takes the name off `leads.tags`,
/// writes the `tag_change_log` row and REVERSES the pending `affiliate_commissions` row a
/// product-linked tag produced (see `reverse_affiliate_on_tags`). The served modal's per-tag "×"
/// calls exactly this.
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

    let triggered_by = req
        .triggered_by
        .clone()
        .unwrap_or_else(|| "manual".to_string());
    let (current_tags, rules_applied) =
        apply_lead_tags(&state, tenant_id, id, &req.tags, &req.remove, &triggered_by).await?;

    Ok(Json(json!({
        "message": "Tags updated",
        "tags": current_tags,
        "rules_applied": rules_applied
    })))
}

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
    /// The Leads screen's search box. Without it the operator searched the table, pressed Export,
    /// and got every lead in the workspace — a file that does not match the rows in front of them.
    pub q: Option<String>,
}

/// One RFC-4180 field: quote it when it carries a delimiter, a quote or a newline, and double any
/// embedded quote. A lead's `notes` is free text, so an unescaped comma there would shift every
/// later column of that row by one — the classic broken export.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// `GET /api/v1/leads/export?format=csv` — every lead of the caller's own workspace, as a file.
///
/// The route used to answer a JSON body for EVERY format, echoing `"format": "csv"` back when that
/// was what the caller asked for: anyone who saved the response as `.csv` got a JSON document, so
/// the export existed in name only — and the console had no control that called it at all. A
/// feature flag (`has_import_export`) is enforced here, which is why the console has to surface the
/// plan refusal rather than pretending the download worked.
///
/// `format=csv` now streams a real file with a download name; the JSON shape stays the default so
/// an existing caller is untouched.
pub async fn export_leads(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> AppResult<Response> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    features::enforce_feature_flag(&state, tenant_id, "has_import_export", "Lead export").await?;

    // The screen's search box narrows the table client-side; the FILE has to narrow the same way or
    // the operator gets rows they had filtered out. An empty box means "everything", exactly as the
    // table shows it.
    let search = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{s}%"));

    let leads = sqlx::query_as::<_, Lead>(
        "SELECT * FROM leads
         WHERE tenant_id = $1
           AND ($2::text IS NULL
                OR name ILIKE $2 OR email ILIKE $2 OR company ILIKE $2
                OR phone ILIKE $2 OR notes ILIKE $2)
         ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .bind(search)
    .fetch_all(&state.pool)
    .await?;

    let wanted = query
        .format
        .as_deref()
        .unwrap_or("json")
        .trim()
        .to_ascii_lowercase();

    if wanted == "csv" {
        let mut out = String::with_capacity(leads.len() * 128 + 64);
        out.push_str("Name,Email,Phone,Company,Source,Stage,Status,Score,Notes,Created\n");
        for l in &leads {
            let name = l
                .name
                .clone()
                .or_else(|| match (&l.first_name, &l.last_name) {
                    (Some(f), Some(la)) => Some(format!("{f} {la}")),
                    (Some(f), None) => Some(f.clone()),
                    (None, Some(la)) => Some(la.clone()),
                    (None, None) => None,
                })
                .unwrap_or_default();
            let row = [
                csv_field(&name),
                csv_field(l.email.as_deref().unwrap_or("")),
                csv_field(l.phone.as_deref().unwrap_or("")),
                csv_field(l.company.as_deref().unwrap_or("")),
                csv_field(l.source.as_deref().unwrap_or("")),
                csv_field(l.stage.as_deref().unwrap_or("")),
                csv_field(l.status.as_deref().unwrap_or("")),
                l.score.map(|s| s.to_string()).unwrap_or_default(),
                csv_field(l.notes.as_deref().unwrap_or("")),
                l.created_at.to_rfc3339(),
            ];
            out.push_str(&row.join(","));
            out.push('\n');
        }

        let filename = format!("leads-{}.csv", Utc::now().format("%Y-%m-%d"));
        return Ok((
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                ),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            out,
        )
            .into_response());
    }

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
    }))
    .into_response())
}

// ---------------------------------------------------------------------------
// CSV lead import (2026-10-18)
// ---------------------------------------------------------------------------

/// One import call accepts at most this many data rows. The file is validated in full and then
/// written in ONE transaction, so the cap bounds a single request; a larger file is REFUSED by name
/// rather than half-imported.
const IMPORT_MAX_ROWS: usize = 2000;

#[derive(Deserialize)]
pub struct ImportLeadsRequest {
    /// The CSV text exactly as the file holds it (the console reads the file and posts its text).
    pub csv: String,
    /// Applies to every imported row when given; otherwise the file's own `Source` column, else
    /// `import`.
    pub source: Option<String>,
}

/// One row of the file, already mapped and validated, waiting for its INSERT.
struct ImportRow {
    first_name: Option<String>,
    last_name: Option<String>,
    name: String,
    email: Option<String>,
    phone: Option<String>,
    company: Option<String>,
    source: Option<String>,
    stage: Option<String>,
    status: Option<String>,
    score: Option<i32>,
    tags: Option<serde_json::Value>,
    notes: Option<String>,
}

/// Split RFC-4180 CSV text into records of fields. A quoted field may hold commas, newlines and
/// doubled quotes — the export beside this writes exactly that shape (`csv_field`), so a file this
/// app exported must re-import unchanged. Hand-written because this crate has no `csv` dependency
/// and a new one is not worth 40 lines.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => record.push(std::mem::take(&mut field)),
            '\r' if !in_quotes => {}
            '\n' if !in_quotes => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if !field.trim().is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// Column lookup by any of several header spellings: the export's own names and the usual CRM
/// imports. The match is case-insensitive and ignores spaces, `_` and `-`, so "First Name",
/// "first_name" and "FirstName" are one column.
fn column_of(headers: &[String], names: &[&str]) -> Option<usize> {
    headers.iter().position(|h| {
        let key: String = h
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !matches!(c, ' ' | '_' | '-'))
            .collect();
        names.contains(&key.as_str())
    })
}

/// `POST /api/v1/leads/import` — put a CSV file's leads into the caller's own workspace.
///
/// WHY THIS EXISTS: the Agency plan advertises `has_import_export`, and until today only the EXPORT
/// half was mounted — `GET /api/v1/leads/export` had a route, a writer and (since the export button
/// shipped) a caller, while nothing anywhere accepted a file. A customer who bought the plan for
/// "import/export" could only get data OUT. This is the other direction, and it round-trips the
/// export's own header (`Name,Email,Phone,Company,Source,Stage,Status,Score,Notes`).
///
/// CONTRACT: the file is read and validated in full FIRST, then written in one transaction. Rows
/// that cannot be stored are reported by line number and skipped; a plan that cannot hold the whole
/// file is refused BEFORE anything is written, with the numbers that decided it. An import never
/// half-lands and never partially charges a limit.
///
/// Deliberate omission, matching `create_lead`'s validation but not its fan-out: no per-row
/// `lead.created` webhook and no per-row WorkflowSwift push. Both are per-lead external calls and a
/// 2000-row file would fan out 2000 of them; a bulk import is a data load, not 2000 new events.
pub async fn import_leads(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ImportLeadsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    // The same entitlement `export_leads` enforces — this is the other half of the same feature, so
    // one gate, one message.
    features::enforce_feature_flag(&state, tenant_id, "has_import_export", "Lead import").await?;

    let records = parse_csv(&req.csv);
    if records.len() < 2 {
        return Err(AppError::UnprocessableEntity(
            "The file has no data rows. The first line must be a header row — Name, Email, Phone, \
             Company, Source, Stage, Status, Score, Notes (any order) — and every following line one lead."
                .into(),
        ));
    }

    let headers: Vec<String> = records[0].clone();
    let i_name = column_of(&headers, &["name", "fullname", "contactname", "contact"]);
    let i_first = column_of(&headers, &["firstname", "givenname"]);
    let i_last = column_of(&headers, &["lastname", "surname", "familyname"]);
    let i_email = column_of(&headers, &["email", "emailaddress", "mail"]);
    let i_phone = column_of(
        &headers,
        &["phone", "phonenumber", "mobile", "tel", "telephone", "cell"],
    );
    let i_company = column_of(
        &headers,
        &[
            "company",
            "companyname",
            "organization",
            "organisation",
            "business",
        ],
    );
    let i_source = column_of(&headers, &["source", "leadsource", "channel"]);
    let i_stage = column_of(&headers, &["stage", "pipelinestage"]);
    let i_status = column_of(&headers, &["status"]);
    let i_score = column_of(&headers, &["score"]);
    let i_notes = column_of(&headers, &["notes", "note", "comments", "comment", "memo"]);
    let i_tags = column_of(&headers, &["tags", "tag", "labels", "label"]);

    if i_name.is_none() && i_first.is_none() && i_email.is_none() {
        return Err(AppError::UnprocessableEntity(format!(
            "No name and no email column found, so no row could become a lead. The first row is the \
             header — name the columns Name, Email, Phone, Company, Source, Stage, Status, Score, \
             Notes. Headers seen: {}",
            headers.join(", ")
        )));
    }

    let data_rows = records.len() - 1;
    if data_rows > IMPORT_MAX_ROWS {
        return Err(AppError::UnprocessableEntity(format!(
            "This file has {data_rows} data rows; one import accepts at most {IMPORT_MAX_ROWS}. \
             Split the file and import the parts."
        )));
    }

    // Every address already in this workspace, so re-importing the same file cannot duplicate it.
    let mut seen: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT lower(email) FROM leads WHERE tenant_id = $1 AND email IS NOT NULL",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .collect();

    let source_override = req
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut prepared: Vec<ImportRow> = Vec::new();
    let mut duplicates: i64 = 0;
    let mut invalid: i64 = 0;
    let mut errors: Vec<serde_json::Value> = Vec::new();

    for (idx, row) in records.iter().skip(1).enumerate() {
        let line = idx + 2; // the header is line 1, and the operator counts from 1
        if row.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        let cell = |i: Option<usize>| {
            i.and_then(|i| row.get(i))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        let first_name = cell(i_first);
        let last_name = cell(i_last);
        let email = cell(i_email);
        let phone = cell(i_phone);
        let company = cell(i_company);
        let stage = cell(i_stage);
        let status = cell(i_status);
        let notes = cell(i_notes);
        let row_source = source_override
            .clone()
            .or_else(|| cell(i_source))
            .or_else(|| Some("import".to_string()));
        let display_name = cell(i_name).or_else(|| {
            let joined = format!(
                "{} {}",
                first_name.as_deref().unwrap_or(""),
                last_name.as_deref().unwrap_or("")
            )
            .trim()
            .to_string();
            (!joined.is_empty()).then_some(joined)
        });

        let mut reason: Option<String> = None;
        if display_name.is_none() && email.is_none() {
            reason = Some("no name and no email on this row".to_string());
        }
        if reason.is_none() {
            for (field, value, cap) in [
                ("name", display_name.as_deref(), lead_fields::TEXT_CHARS),
                ("email", email.as_deref(), lead_fields::TEXT_CHARS),
                ("phone", phone.as_deref(), lead_fields::PHONE_CHARS),
                ("company", company.as_deref(), lead_fields::TEXT_CHARS),
                ("source", row_source.as_deref(), lead_fields::SOURCE_CHARS),
                ("stage", stage.as_deref(), lead_fields::STAGE_CHARS),
                ("status", status.as_deref(), lead_fields::STATUS_CHARS),
            ] {
                if let Err(e) = lead_fields::check(field, value, cap) {
                    reason = Some(e);
                    break;
                }
            }
        }
        if reason.is_none() {
            if let Some(e) = &email {
                if !e.contains('@') || e.chars().any(char::is_whitespace) {
                    reason = Some(format!("'{e}' is not an email address"));
                }
            }
        }
        if let Some(msg) = reason {
            invalid += 1;
            if errors.len() < 10 {
                errors.push(json!({ "line": line, "reason": msg }));
            }
            continue;
        }

        if let Some(e) = &email {
            let key = e.to_ascii_lowercase();
            if seen.contains(&key) {
                duplicates += 1;
                continue;
            }
            seen.insert(key);
        }

        let tags: Option<serde_json::Value> = cell(i_tags).map(|raw| {
            let parts: Vec<serde_json::Value> = raw
                .split([';', '|', ','])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect();
            serde_json::Value::Array(parts)
        });

        prepared.push(ImportRow {
            first_name,
            last_name,
            name: display_name.unwrap_or_default(),
            email,
            phone,
            company,
            source: row_source,
            stage,
            status,
            score: cell(i_score).and_then(|s| s.parse::<i32>().ok()),
            tags,
            notes,
        });
    }

    // The SAME numeric limit `create_lead` enforces, read through the ONE resolver and applied to the
    // whole file BEFORE a single row is written: an import must not jump the plan, and must not leave
    // half a file behind when it hits the ceiling.
    if let Some(headroom) = features::remaining_headroom(&state, tenant_id, "max_leads").await? {
        if prepared.len() as i64 > headroom {
            return Err(AppError::UpgradeRequired(format!(
                "Your plan has room for {headroom} more leads and this file would add {}. Nothing \
                 was imported — upgrade the plan, or import {} rows or fewer.",
                prepared.len(),
                headroom
            )));
        }
    }

    let created_by = Uuid::parse_str(&auth.user_id).ok();
    let mut tx = state.pool.begin().await?;
    for r in &prepared {
        sqlx::query(
            r#"INSERT INTO leads (id, tenant_id, first_name, last_name, name, email, phone, company,
                                  source, stage, status, score, tags, custom_fields, notes, created_by)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, COALESCE($10, 'New'),
                       COALESCE($11, 'new'), $12, $13, NULL, $14, $15)"#,
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(&r.first_name)
        .bind(&r.last_name)
        .bind(&r.name)
        .bind(&r.email)
        .bind(&r.phone)
        .bind(&r.company)
        .bind(&r.source)
        .bind(&r.stage)
        .bind(&r.status)
        .bind(r.score)
        .bind(&r.tags)
        .bind(&r.notes)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(Json(json!({
        "imported": prepared.len(),
        "duplicates": duplicates,
        "invalid": invalid,
        "data_rows": data_rows,
        "errors": errors,
        "message": format!(
            "{} imported, {} already in the workspace, {} could not be read",
            prepared.len(),
            duplicates,
            invalid
        ),
    })))
}
