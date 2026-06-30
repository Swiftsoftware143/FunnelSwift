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
use crate::models::lead::*;
use crate::handlers::workflowswift_push::push_to_workflowswift;
use crate::state::AppState;
use crate::features;

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

    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM leads WHERE tenant_id = $1",
    )
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
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
                    "A lead with email '{}' already exists in this workspace", email
                )));
            }
        }
    }

    let lead_id = Uuid::new_v4();

    sqlx::query(
        r#"INSERT INTO leads (id, tenant_id, name, email, phone, company, source, stage, tags, custom_fields, notes, assigned_to)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(lead_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.company)
    .bind(&req.source)
    .bind(&req.stage)
    .bind(req.tags.map(|t| serde_json::Value::Array(t.into_iter().map(serde_json::Value::String).collect())))
    .bind(&req.custom_fields)
    .bind(&req.notes)
    .bind(req.assigned_to)
    .execute(&state.pool)
    .await?;

    // Best-effort push to WorkflowSwift
    tokio::spawn({
        let pool = state.pool.clone();
        let url = state.workflowswift_url.clone();
        async move {
            push_to_workflowswift(&pool, &url, lead_id, tenant_id).await;
        }
    });

    Ok((StatusCode::CREATED, Json(json!({"id": lead_id, "message": "Lead created"}))))
}

pub async fn get_lead(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Lead>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let lead = sqlx::query_as::<_, Lead>(
        "SELECT * FROM leads WHERE id = $1 AND tenant_id = $2",
    )
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let existing = sqlx::query_as::<_, Lead>(
        "SELECT * FROM leads WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Lead not found".into()))?;

    let name = req.name.clone().or(existing.name.clone()).unwrap_or_default();
    let email = req.email.clone().or(existing.email.clone());
    let phone = req.phone.clone().or(existing.phone.clone());
    let company = req.company.clone().or(existing.company.clone());
    let source = req.source.clone().or(existing.source.clone());
    let status = req.status.clone().or(Some(existing.status.clone())).unwrap_or_else(|| "active".to_string());
    let stage = req.stage.clone().or(existing.stage.clone());
    let score = req.score.or(existing.score);
    let notes = req.notes.or(existing.notes);
    let assigned_to = req.assigned_to.or(existing.assigned_to);
    let tags = req.tags.map(|t| serde_json::Value::Array(t.into_iter().map(serde_json::Value::String).collect()));
    let custom_fields = req.custom_fields.or(existing.custom_fields);

    sqlx::query(
        r#"UPDATE leads SET name=$1, email=$2, phone=$3, company=$4, source=$5, status=$6, stage=$7, score=$8,
           tags=$9, custom_fields=$10, notes=$11, assigned_to=$12, updated_at=NOW() WHERE id=$13 AND tenant_id=$14"#,
    )
    .bind(&name)
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("UPDATE leads SET assigned_to = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    sqlx::query("UPDATE leads SET stage = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
        .bind(&req.stage)
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;

    // Log activity
    sqlx::query(
        "INSERT INTO activity_log (tenant_id, user_id, action, entity_type, entity_id, metadata) VALUES ($1, $2, 'stage_change', 'lead', $3, $4)",
    )
    .bind(tenant_id)
    .bind(&auth.user_id)
    .bind(id.to_string())
    .bind(json!({"new_stage": req.stage}))
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({"message": "Stage updated"})))
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
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let leads = sqlx::query_as::<_, Lead>("SELECT * FROM leads WHERE tenant_id = $1 ORDER BY created_at DESC")
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?;

    let csv_leads: Vec<serde_json::Value> = leads.iter().map(|l| {
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
    }).collect();

    Ok(Json(json!({
        "format": query.format.as_deref().unwrap_or("json"),
        "count": leads.len(),
        "data": csv_leads
    })))
}
