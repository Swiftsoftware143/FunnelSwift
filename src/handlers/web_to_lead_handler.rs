use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateWebToLeadConfig {
    pub name: String,
    pub form_title: Option<String>,
    pub fields: Option<Vec<String>>,
    pub thank_you_message: Option<String>,
    pub redirect_url: Option<String>,
    /// The tag this form stamps onto every lead it captures. David's model: *"that lead form ... gets
    /// assigned to a tag when it's created"*. The column existed and this field did not, so a caller
    /// could send `tag_ids` all day and it was silently dropped on the floor.
    pub tag_ids: Option<Vec<Uuid>>,
}

pub async fn list_web_to_lead_configs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // This SELECTed a form_title column that does not exist (and read a timestamptz into a
    // NaiveDateTime) then hid the failure behind `.unwrap_or_default()`, so the list came
    // back empty for every tenant no matter how many forms they had.
    let rows: Vec<(Uuid, String, Value, bool, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, name, COALESCE(field_mapping, '{}'::jsonb) AS field_mapping, is_active, created_at FROM web_to_lead_configs \
         WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, mapping, is_active, created_at)| {
            json!({
                "id": id.to_string(),
                "name": name,
                "form_title": mapping.get("form_title").cloned().unwrap_or(json!("")),
                "fields": mapping.get("fields").cloned().unwrap_or(json!([])),
                "is_active": is_active,
                "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!(out)))
}
pub async fn create_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<CreateWebToLeadConfig>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // An affiliate is never blocked by the plan's form limit.
    //
    // Measured 2026-10-01: public signup puts every new customer on `kinetic-free`, whose
    // `max_forms` is 0 — and `enforce_feature_limit` reads 0 as "not available on your plan", not as
    // a quantity. So every new affiliate who tried to build the tag-bound form David's model is
    // built on got:
    //     {"error":"Web-to-lead forms is not available on your current plan.","status":402}
    // An affiliate who cannot build a form cannot hand one out, so the programme produced nothing at
    // all for anyone who signed up normally — the screen offered them products to promote and then
    // refused the only tool that could credit them. The form an affiliate hands out is not a plan
    // feature of theirs to buy; it is how the programme works, so it is carved out here rather than
    // by editing what every plan advertises.
    let is_affiliate: bool = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM affiliates WHERE tenant_id = $1 AND is_active = true)",
    )
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await?;
    if !is_affiliate {
        features::enforce_feature_limit(&state, tenant_id, "max_forms", "Web-to-lead forms")
            .await?;
    }
    let fields = payload
        .fields
        .unwrap_or_else(|| vec!["name".to_string(), "email".to_string()]);
    // `web_to_lead_configs` has no form_title/fields columns and mints its own uuid
    // public_key — the previous INSERT named two phantom columns and bound a String into
    // a uuid, so every create returned 500 and the product never worked once.
    let field_mapping = json!({
        "form_title": payload.form_title.as_deref().unwrap_or("Get Started"),
        "fields": fields,
    });
    let public_key: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO web_to_lead_configs (id, tenant_id, name, default_source, field_mapping, tag_ids) \
         VALUES ($1, $2, $3, 'Web Form', $4, $5) RETURNING public_key",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&payload.name)
    .bind(&field_mapping)
    .bind(&payload.tag_ids)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id.to_string(),
            "public_key": public_key.unwrap_or(id).to_string(),
        })),
    ))
}
pub async fn update_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    // This handler used to return {"message":"Config updated"} without touching the
    // database — an edit in the app looked saved and reverted on the next fetch.
    let mut mapping = json!({});
    if let Some(title) = payload.get("form_title") {
        mapping["form_title"] = title.clone();
    }
    if let Some(fields) = payload.get("fields") {
        mapping["fields"] = fields.clone();
    }
    let affected = sqlx::query(
        "UPDATE web_to_lead_configs SET \
           name = COALESCE($3, name), \
           is_active = COALESCE($4, is_active), \
           default_source = COALESCE($5, default_source), \
           field_mapping = field_mapping || $6::jsonb, \
           tag_ids = COALESCE($7, tag_ids), \
           updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(payload.get("name").and_then(|v| v.as_str()))
    .bind(payload.get("is_active").and_then(|v| v.as_bool()))
    .bind(payload.get("default_source").and_then(|v| v.as_str()))
    .bind(&mapping)
    // An empty array CLEARS the binding; sending nothing leaves it alone. Parsed from JSON so a
    // caller can rebind a form to a different tag without recreating it.
    .bind(payload.get("tag_ids").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str())
            .filter_map(|x| Uuid::parse_str(x).ok())
            .collect::<Vec<Uuid>>()
    }))
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("Config not found".into()));
    }
    Ok(Json(json!({"message": "Config updated"})))
}
pub async fn delete_web_to_lead_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("DELETE FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Config deleted"})))
}
pub async fn get_web_to_lead_embed(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let public_key: Option<Uuid> = sqlx::query_scalar(
        "SELECT public_key FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let public_key = public_key.ok_or_else(|| AppError::NotFound("Config not found".into()))?;
    // The public endpoint resolves the tenant from public_key, so a snippet without the key
    // (and pointing at a /embed.js that does not exist) could never capture a lead.
    let embed_code = format!(
        "<form id=\"fsw-wtl-{id}\" onsubmit=\"event.preventDefault();fetch('https://funnelswift.net/api/v1/web-to-lead',{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{public_key:'{public_key}',name:this.name.value,email:this.email.value}})}}).then(()=>this.reset());\">\n  <input name=\"name\" placeholder=\"Name\" required />\n  <input name=\"email\" type=\"email\" placeholder=\"Email\" required />\n  <button type=\"submit\">Send</button>\n</form>"
    );
    Ok(Json(json!({
        "embed_code": embed_code,
        "public_key": public_key.to_string(),
        "endpoint": "https://funnelswift.net/api/v1/web-to-lead",
    })))
}
pub async fn handle_web_to_lead(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Resolve the tenant from the config's public_key — never hardcode or trust a body tenant_id.
    let public_key = payload["public_key"].as_str().unwrap_or("");
    // public_key is a uuid column: binding the raw string made Postgres fail with
    // "operator does not exist: uuid = text", so no form submission ever captured a lead.
    let public_key = Uuid::parse_str(public_key)
        .map_err(|_| AppError::BadRequest("Invalid public_key".into()))?;
    let tenant_id: Option<Uuid> =
        sqlx::query_scalar("SELECT tenant_id FROM web_to_lead_configs WHERE public_key = $1")
            .bind(public_key)
            .fetch_optional(&state.pool)
            .await?;
    let tenant_id = tenant_id.ok_or_else(|| AppError::BadRequest("Unknown public_key".into()))?;
    // ── THE FORM IS THE ATTRIBUTION (David, 2026-10-01) ─────────────────────────────────────────
    // David: *"that lead form that snippet of code or lead form gets assigned to a tag when it's
    // created so that way they can connect it to their own personal landing page"* and *"they are
    // permanently assigned to that affiliate no cookies or anything"*.
    //
    // MEASURED BEFORE THIS: the config already HAD a `tag_ids` column and nothing ever read it. This
    // handler looked up the tenant and inserted the lead — that was all. So an affiliate could build a
    // form, embed it on their own landing page, and every lead it captured arrived UNTAGGED, with no
    // affiliate behind it and invisible to every affiliate view. The single step that makes the whole
    // model work did nothing at all.
    let cfg: Option<(Option<Vec<Uuid>>, Option<String>, Option<bool>)> = sqlx::query_as(
        "SELECT tag_ids, default_source, is_active FROM web_to_lead_configs WHERE public_key = $1",
    )
    .bind(public_key)
    .fetch_optional(&state.pool)
    .await?;
    let (tag_ids, default_source, is_active) = cfg.unwrap_or((None, None, None));
    if is_active == Some(false) {
        // An inactive form was still capturing leads.
        return Err(AppError::BadRequest("This form is switched off".into()));
    }
    let tag_ids: Vec<Uuid> = tag_ids.unwrap_or_default();

    // The affiliate anchor. One affiliate per customer (migration 069 makes that unique), so a tenant
    // resolves to at most one. Their USER id is what matters: `attribute_affiliate_on_tags` reads
    // `leads.created_by`, so a lead whose created_by is NULL can never be credited no matter how it
    // was tagged.
    let anchor: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, user_id FROM affiliates WHERE tenant_id = $1 AND is_active = true LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    let affiliate_user: Option<Uuid> = anchor.as_ref().and_then(|(_, u)| *u);

    // Stamp the tag NAMES onto the lead — that is the shape `leads.tags` uses (a JSON array of names,
    // measured in lead_handler::assign_lead_tags) — and keep the IDS for the credit below.
    let tag_names: Vec<String> = if tag_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar("SELECT name FROM tags WHERE id = ANY($1) ORDER BY name")
            .bind(&tag_ids)
            .fetch_all(&state.pool)
            .await?
    };
    let tags_json = serde_json::Value::Array(
        tag_names
            .iter()
            .map(|n| serde_json::Value::String(n.clone()))
            .collect(),
    );

    // A lead captured through an affiliate's form IS an affiliate lead — that is what the admin's
    // Affiliate Leads view reads (`source = 'affiliate'`). A direct capture keeps the form's own
    // default source and has no affiliate behind it, so it shows as unattributed / "system".
    let source = if affiliate_user.is_some() {
        "affiliate".to_string()
    } else {
        default_source
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "web".to_string())
    };

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO leads (id, tenant_id, name, email, phone, company, source, status, created_by, tags) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'new', $8, $9)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(payload["name"].as_str().unwrap_or(""))
    .bind(payload["email"].as_str().unwrap_or(""))
    // phone and company were being dropped on the floor even when the form sent them.
    .bind(payload["phone"].as_str())
    .bind(payload["company"].as_str())
    .bind(&source)
    .bind(affiliate_user)
    .bind(&tags_json)
    .execute(&state.pool)
    .await?;

    // THE KEYSTONE STEP: stamping the tag is what CREDITS the affiliate. Without this call the tag
    // would be pure decoration — the lead would carry it and no commission would ever exist.
    if !tag_ids.is_empty() {
        if let Err(e) =
            crate::tag_logic::attribute_affiliate_on_tags(&state.pool, id, &tag_ids).await
        {
            // The lead is already stored, so a failed credit must be loud rather than losing the lead.
            tracing::error!(
                lead_id = %id,
                error = %e,
                "web-to-lead: tag stored but affiliate attribution FAILED — this lead credits nobody"
            );
        }
    }

    // INBOUND CoreSwift push (fleet standard 2026-09-20 §R2): every captured opt-in must be
    // able to land in CoreSwift as a contact. Fire-and-forget: the visitor's submission above
    // already succeeded, and this is a no-op when the tenant has no `coreswift` BYOK key.
    crate::coreswift::spawn_lead_push(
        state.pool.clone(),
        tenant_id,
        crate::coreswift::LeadPayload {
            email: payload
                .get("email")
                .and_then(|v| v.as_str())
                .map(String::from),
            phone: payload
                .get("phone")
                .and_then(|v| v.as_str())
                .map(String::from),
            name: payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from),
            company: payload
                .get("company")
                .and_then(|v| v.as_str())
                .map(String::from),
            source: Some("web_to_lead".to_string()),
            ..Default::default()
        },
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id.to_string(),
            "message": "Lead captured",
            "source": source,
            "tags": tag_names,
            "attributed_to_affiliate": affiliate_user.is_some() && !tag_ids.is_empty(),
        })),
    ))
}
