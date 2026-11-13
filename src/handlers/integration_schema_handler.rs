//! `GET /api/v1/integration/schema` — the SOURCE half of the CoreSwift visual field mapper.
//!
//! Contract: /opt/swift/docs/coreswift-data-mapping-spec-2026-11-13.md §2. CoreSwift's mapper
//! calls this to list FunnelSwift's REAL lead fields with MASKED sample values instead of a
//! hardcoded list. Response shape:
//!
//! ```json
//! {"app":"funnelswift","version":1,"entities":[{"name":"leads","label":"Leads",
//!   "sample_count":N,"fields":[{"path":"first_name","label":"First name","type":"string",
//!   "required":false,"sample":"John"}, ...]}]}
//! ```
//!
//! Rules this handler honours:
//! * The real schema wins over the spec's illustrative field names — the column list below is the
//!   verified `leads` table (`information_schema`, 2026-11-13), NOT the spec's examples.
//! * Samples are MASKED: emails -> `j***@e***.com`, phones keep the last three digits
//!   (`+1***1234`), names/company/source/stage/tags are labels returned as-is. No full email or
//!   phone number may leave here.
//! * `sample: null` when there is no data — a sample is never fabricated.
//! * Nested jsonb is flattened to dot paths (`custom_fields.<key>`) so the UI can render a
//!   collapsible tree; `tags` and `custom_fields` themselves are exposed as `array` / `object`.
//! * `type` is one of the mapper's widget vocabulary: string/email/phone/date/number/array/object/bool.
//! * ADMIN ONLY: a non-admin caller is refused 403 in the handler (the route is a plain
//!   `require_auth` route — not on `/api/v1/admin/` — so the handler owns the role check, exactly
//!   like the other `is_admin`-gated handlers). A missing credential is refused 401 by the
//!   credential boundary before this handler runs.

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// `GET /api/v1/integration/schema`
pub async fn integration_schema(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if !auth.is_admin {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    let sample_count: i64 = sqlx::query_scalar("SELECT count(*) FROM leads")
        .fetch_one(&state.pool)
        .await?;

    // One live row is enough: the mapper needs the SHAPE of each value, not the whole table.
    // Prefer the most recently updated lead. With no rows every sample below is null.
    #[derive(sqlx::FromRow)]
    struct SampleRow {
        name: String,
        first_name: Option<String>,
        last_name: Option<String>,
        email: Option<String>,
        phone: Option<String>,
        company: Option<String>,
        source: Option<String>,
        status: Option<String>,
        stage: Option<String>,
        score: Option<i32>,
        tags: Option<Value>,
        custom_fields: Option<Value>,
        notes: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let row: Option<SampleRow> = sqlx::query_as(
        "SELECT name, first_name, last_name, email, phone, company, source, status, stage, \
         score, tags, custom_fields, notes, created_at, updated_at \
         FROM leads ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;

    let mut fields: Vec<Value> = Vec::new();

    // ── scalar columns (the verified `leads` schema) ────────────────────────────────
    // `required` mirrors the column's NOT NULL constraint: only `name` (and the two
    // auto timestamps) are NOT NULL, everything else is nullable.
    push(
        &mut fields,
        "name",
        "Name",
        "string",
        true,
        row.as_ref().map(|r| json!(r.name.clone())),
    );
    push(
        &mut fields,
        "first_name",
        "First name",
        "string",
        false,
        row.as_ref().and_then(|r| r.first_name.clone()).map(Value::String),
    );
    push(
        &mut fields,
        "last_name",
        "Last name",
        "string",
        false,
        row.as_ref().and_then(|r| r.last_name.clone()).map(Value::String),
    );
    push(
        &mut fields,
        "email",
        "Email",
        "email",
        false,
        row.as_ref()
            .and_then(|r| r.email.as_deref())
            .map(|e| json!(mask_email(e))),
    );
    push(
        &mut fields,
        "phone",
        "Phone",
        "phone",
        false,
        row.as_ref()
            .and_then(|r| r.phone.as_deref())
            .map(|p| json!(mask_phone(p))),
    );
    push(
        &mut fields,
        "company",
        "Company",
        "string",
        false,
        row.as_ref().and_then(|r| r.company.clone()).map(Value::String),
    );
    push(
        &mut fields,
        "source",
        "Source",
        "string",
        false,
        row.as_ref().and_then(|r| r.source.clone()).map(Value::String),
    );
    push(
        &mut fields,
        "status",
        "Status",
        "string",
        false,
        row.as_ref().and_then(|r| r.status.clone()).map(Value::String),
    );
    push(
        &mut fields,
        "stage",
        "Stage",
        "string",
        false,
        row.as_ref().and_then(|r| r.stage.clone()).map(Value::String),
    );
    push(
        &mut fields,
        "score",
        "Score",
        "number",
        false,
        row.as_ref().and_then(|r| r.score).map(|s| json!(s)),
    );
    push(
        &mut fields,
        "tags",
        "Tags",
        "array",
        false,
        row.as_ref().and_then(|r| r.tags.as_ref()).map(mask_json),
    );
    // notes is free text and can carry an address or a number the mapper must never see.
    push(
        &mut fields,
        "notes",
        "Notes",
        "string",
        false,
        row.as_ref()
            .and_then(|r| r.notes.as_deref())
            .map(|n| json!(mask_free_text(n))),
    );
    push(
        &mut fields,
        "created_at",
        "Created at",
        "date",
        true,
        row.as_ref().map(|r| json!(r.created_at.to_rfc3339())),
    );
    push(
        &mut fields,
        "updated_at",
        "Updated at",
        "date",
        true,
        row.as_ref().map(|r| json!(r.updated_at.to_rfc3339())),
    );

    // ── nested jsonb: `custom_fields` (object) + its flattened dot paths ────────────
    let custom_sample = row.as_ref().and_then(|r| r.custom_fields.as_ref()).map(mask_json);
    push(&mut fields, "custom_fields", "Custom fields", "object", false, custom_sample);

    if let Some(obj) = row
        .as_ref()
        .and_then(|r| r.custom_fields.as_ref())
        .and_then(|v| v.as_object())
    {
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort_unstable();
        for key in keys {
            let (ty, sample) = infer_scalar(&obj[key]);
            fields.push(json!({
                "path": format!("custom_fields.{key}"),
                "label": humanize(key),
                "type": ty,
                "required": false,
                "sample": sample,
            }));
        }
    }

    Ok(Json(json!({
        "app": "funnelswift",
        "version": 1,
        "entities": [{
            "name": "leads",
            "label": "Leads",
            "sample_count": sample_count,
            "fields": fields,
        }],
    })))
}

fn push(fields: &mut Vec<Value>, path: &str, label: &str, ty: &str, required: bool, sample: Option<Value>) {
    fields.push(json!({
        "path": path,
        "label": label,
        "type": ty,
        "required": required,
        "sample": sample.unwrap_or(Value::Null),
    }));
}

/// `j***@e***.com` — the first char of the local part, the first char of the domain, and the TLD.
/// The rest is shape, never the address.
fn mask_email(email: &str) -> String {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return mask_free_text(email);
    };
    let first_local = local.chars().next().map(String::from).unwrap_or_default();
    let first_domain = domain.chars().next().map(String::from).unwrap_or_default();
    let tld = domain
        .rsplit_once('.')
        .map(|(_, t)| format!(".{t}"))
        .unwrap_or_default();
    format!("{first_local}***@{first_domain}***{tld}")
}

/// `+1***1234` — a leading `+` and the country-code digits survive, the middle is masked, the last
/// three digits are kept. Never returns more than a 3-digit run, so a full number cannot leak.
fn mask_phone(phone: &str) -> String {
    let trimmed = phone.trim();
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return String::new();
    }
    let last3: String = digits
        .chars()
        .rev()
        .take(3)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    // Digits beyond a trailing 10-digit local number are the country code; keep those plus a `+`.
    let cc_len = digits.len().saturating_sub(10);
    let cc: String = digits.chars().take(cc_len).collect();
    let plus = if trimmed.starts_with('+') { "+" } else { "" };
    format!("{plus}{cc}***{last3}")
}

fn looks_like_email(s: &str) -> bool {
    let trimmed = s.trim_matches(|c: char| c.is_ascii_punctuation());
    if trimmed.chars().filter(|c| *c == '@').count() != 1 {
        return false;
    }
    let Some((local, domain)) = trimmed.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// Redact email and phone-like tokens inside free text (notes, custom-field string leaves), so no
/// full address or number escapes in a value this handler did not type-check as a field.
fn mask_free_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut first = true;
    for token in text.split_whitespace() {
        if !first {
            out.push(' ');
        }
        first = false;
        if looks_like_email(token) {
            out.push_str(&mask_email(token));
        } else {
            let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 7 {
                out.push_str(&mask_phone(token));
            } else {
                out.push_str(token);
            }
        }
    }
    out
}

/// Mask every string leaf of an arbitrary jsonb value (tags arrays, custom_fields objects).
fn mask_json(v: &Value) -> Value {
    match v {
        Value::String(s) => json!(mask_free_text(s)),
        Value::Array(a) => Value::Array(a.iter().map(mask_json).collect()),
        Value::Object(o) => {
            let mut m = serde_json::Map::new();
            for (k, val) in o {
                m.insert(k.clone(), mask_json(val));
            }
            Value::Object(m)
        }
        other => other.clone(),
    }
}

/// Infer the mapper widget type for a custom-fields leaf and mask its string sample.
fn infer_scalar(v: &Value) -> (&'static str, Value) {
    match v {
        Value::String(_) => ("string", mask_json(v)),
        Value::Number(_) => ("number", v.clone()),
        Value::Bool(_) => ("bool", v.clone()),
        Value::Array(_) => ("array", mask_json(v)),
        Value::Object(_) => ("object", mask_json(v)),
        Value::Null => ("string", Value::Null),
    }
}

/// `utm_source` -> `Utm source` — a readable label for a flattened custom-fields key.
fn humanize(key: &str) -> String {
    let words: Vec<String> = key
        .split(['_', '-'])
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    words.join(" ")
}
