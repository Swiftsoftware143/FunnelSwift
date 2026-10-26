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
    // `tag_ids` is read here (2026-10-26): it is the form's ATTRIBUTION, and until the list
    // carried it the console could neither show which tag a live form stamps nor preselect it on
    // Edit — the operator had to remember an invisible binding.
    let rows: Vec<(
        Uuid,
        String,
        Value,
        bool,
        chrono::DateTime<chrono::Utc>,
        Option<Vec<Uuid>>,
    )> = sqlx::query_as(
        "SELECT id, name, COALESCE(field_mapping, '{}'::jsonb) AS field_mapping, is_active, created_at, tag_ids FROM web_to_lead_configs \
         WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, mapping, is_active, created_at, tag_ids)| {
            json!({
                "id": id.to_string(),
                "name": name,
                "form_title": mapping.get("form_title").cloned().unwrap_or(json!("")),
                "fields": mapping.get("fields").cloned().unwrap_or(json!([])),
                // The panel's own thank-you / redirect inputs (2026-10-26). They were accepted
                // by the editor and stored nowhere, so a saved form always said the default and
                // never redirected.
                "thank_you_message": mapping
                    .get("thank_you_message")
                    .cloned()
                    .unwrap_or(json!("")),
                "redirect_url": mapping
                    .get("redirect_url")
                    .cloned()
                    .unwrap_or(json!("")),
                "tag_ids": tag_ids
                    .unwrap_or_default()
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<String>>(),
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
    // `web_to_lead_configs.name` is `varchar(255)`; measured live 2026-10-17, a 300-character
    // name answered a raw 500 from the INSERT. Refused here, naming the field.
    crate::security::lead_fields::check(
        "name",
        Some(&payload.name),
        crate::security::lead_fields::TEXT_CHARS,
    )
    .map_err(AppError::UnprocessableEntity)?;
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
    // The two fields the panel has always offered and this INSERT always threw away (measured
    // 2026-10-26: created with both, read back neither). Trimmed and bounded, because they land
    // in a jsonb document and are echoed into the public snippet.
    let clip =
        |v: Option<&str>| -> String { v.map(str::trim).unwrap_or("").chars().take(500).collect() };
    let field_mapping = json!({
        "form_title": payload.form_title.as_deref().unwrap_or("Get Started"),
        "fields": fields,
        "thank_you_message": clip(payload.thank_you_message.as_deref()),
        "redirect_url": clip(payload.redirect_url.as_deref()),
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
    // The same two columns on the edit path (`name`, `default_source`, both `varchar(255)`).
    for field in ["name", "default_source"] {
        crate::security::lead_fields::check(
            field,
            payload.get(field).and_then(|v| v.as_str()),
            crate::security::lead_fields::TEXT_CHARS,
        )
        .map_err(AppError::UnprocessableEntity)?;
    }
    // This handler used to return {"message":"Config updated"} without touching the
    // database — an edit in the app looked saved and reverted on the next fetch.
    let mut mapping = json!({});
    if let Some(title) = payload.get("form_title") {
        mapping["form_title"] = title.clone();
    }
    if let Some(fields) = payload.get("fields") {
        mapping["fields"] = fields.clone();
    }
    // Edit path for the same two fields (2026-10-26). `field_mapping || $6::jsonb` merges, so a
    // key the caller omits keeps its stored value; an explicit "" clears it.
    for key in ["thank_you_message", "redirect_url"] {
        if let Some(v) = payload.get(key).and_then(|v| v.as_str()) {
            mapping[key] = json!(v.trim().chars().take(500).collect::<String>());
        }
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
    let result = sqlx::query("DELETE FROM web_to_lead_configs WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Config not found".into()));
    }
    Ok(Json(json!({"message": "Config deleted"})))
}
pub async fn get_web_to_lead_embed(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let row: Option<(Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT public_key, COALESCE(field_mapping, '{}'::jsonb) FROM web_to_lead_configs \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    let (public_key, mapping) = row.unwrap_or((None, json!({})));
    let public_key = public_key.ok_or_else(|| AppError::NotFound("Config not found".into()))?;
    // The snippet is the ONLY place a captured visitor is answered, so the form's own thank-you
    // text and redirect belong in it (2026-10-26). Both were stored nowhere before, and the
    // snippet hardcoded one sentence and never redirected.
    let thanks = mapping
        .get("thank_you_message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let redirect = mapping
        .get("redirect_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let thanks_js = if thanks.is_empty() {
        "\"Thanks - we got your details.\"".to_string()
    } else {
        serde_json::to_string(thanks).unwrap_or_else(|_| "\"Thanks - we got your details.\"".into())
    };
    // A real redirect only for an absolute http(s) URL: the text is operator-supplied.
    let redirect_js = if redirect.starts_with("http://") || redirect.starts_with("https://") {
        let q = serde_json::to_string(redirect).unwrap_or_default();
        "setTimeout(function(){location.href=".to_string() + &q + ";},1200);"
    } else {
        String::new()
    };
    // The public endpoint resolves the tenant from public_key, so a snippet without the key
    // (and pointing at a /embed.js that does not exist) could never capture a lead.
    let embed_code = format!(
        "<div id=\"fsw-wtl-{id}\">\n<form id=\"fsw-wtl-{id}-form\" onsubmit=\"return fswWtl(event,this,'{public_key}')\">\n  <input name=\"name\" placeholder=\"Name\" required />\n  <input name=\"email\" type=\"email\" placeholder=\"Email\" required />\n  <button type=\"submit\">Send</button>\n</form>\n<p class=\"fsw-wtl-msg\" style=\"font:14px/1.4 system-ui,sans-serif;margin:6px 0\"></p>\n</div>\n"
    ) + r#"<script>
function fswWtl(e,f,k){
  e.preventDefault();
  var m=f.parentNode.querySelector('.fsw-wtl-msg');
  m.textContent='Sending...';m.style.color='#475569';
  fetch('https://funnelswift.net/api/v1/web-to-lead',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({public_key:k,name:f.name.value,email:f.email.value})})
  .then(function(r){return r.json().catch(function(){return{};}).then(function(j){
    if(!r.ok){throw new Error(j.error||('Error '+r.status));}
    f.reset();m.style.color='#15803d';m.textContent=__THANKS__;__REDIRECT__
  });})
  .catch(function(err){m.style.color='#b91c1c';m.textContent='Sorry, that did not send: '+err.message;});
  return false;
}
</script>"#;
    let embed_code = embed_code
        .replace("__THANKS__", &thanks_js)
        .replace("__REDIRECT__", &redirect_js);
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
    // The plan's lead ceiling binds THIS door too, not only the panel's create (`lead_handler`).
    // Measured live 2026-10-17 before this line: a tenant on `capture-free` (max_leads 100) holding
    // 100 leads answered `POST /api/v1/web-to-lead` **201** and wrote a 101st row — the public form
    // is the door most leads actually arrive through, so leaving it ungated made the paid ceiling
    // decorative. Same helper, same key and same 402 sentence the panel's create uses.
    features::enforce_feature_limit(&state, tenant_id, "max_leads", "Leads").await?;
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
    // A stranger filled this form: an over-long company or a pasted phone must not lose the lead
    // with a raw 500, so each value is cut to what its column holds instead of being refused
    // (`security::lead_fields::fit`). Measured live 2026-10-17: the 255/50 columns were
    // unguarded, so `POST /api/v1/leads` with a 300-character name answered 500.
    .bind(crate::security::lead_fields::fit(
        payload["name"].as_str().unwrap_or(""),
        crate::security::lead_fields::TEXT_CHARS,
    ))
    .bind(crate::security::lead_fields::fit(
        payload["email"].as_str().unwrap_or(""),
        crate::security::lead_fields::TEXT_CHARS,
    ))
    // phone and company were being dropped on the floor even when the form sent them.
    .bind(crate::security::lead_fields::fit_opt(
        payload["phone"].as_str(),
        crate::security::lead_fields::PHONE_CHARS,
    ))
    .bind(crate::security::lead_fields::fit_opt(
        payload["company"].as_str(),
        crate::security::lead_fields::TEXT_CHARS,
    ))
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

    // Outbound webhook delivery (kanban t_431faa99): `lead.created` fires the moment the lead row
    // exists, for every ACTIVE webhook in this workspace registered for `lead.created`.
    // Fire-and-forget on purpose — the visitor's submission is already stored and must not wait on
    // a third party that may be slow or gone.
    crate::webhooks::spawn(
        state.pool.clone(),
        tenant_id,
        crate::webhooks::LEAD_CREATED,
        crate::webhooks::lead_created(
            id,
            payload["name"].as_str(),
            payload["email"].as_str(),
            payload["phone"].as_str(),
            payload["company"].as_str(),
            Some(&source),
            &tag_names,
        ),
    );

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
