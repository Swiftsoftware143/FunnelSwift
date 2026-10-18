use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::security::display_name::slugify;
use crate::state::AppState;
use crate::templates::html_escape;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

fn tenant_of(auth: &AuthUser) -> Uuid {
    Uuid::parse_str(&auth.tenant_id).unwrap_or_default()
}

pub async fn list_funnels(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    // The four page strings ship with the LIST because the console's page editor prefills from the
    // row it already holds: one request, and the form cannot disagree with the row the customer
    // clicked.
    let rows: Vec<(
        Uuid,
        String,
        String,
        chrono::NaiveDateTime,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT id, name, COALESCE(slug,''), created_at, headline, subheadline, cta_label, thank_you \
         FROM funnels WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| tracing::error!("list_funnels query failed: {:?}", e))
    .unwrap_or_default();
    // NAMED fields, not the tuple's positional order. The console that renders this list is the app
    // host's Funnels screen, and a JSON array of arrays forces it to index by position — one added
    // column in the SELECT and every row silently shows the wrong cell. (`rows` was a tuple until
    // 2026-10-17; nothing consumed it, so this is a shape fix, not a breaking change.)
    let funnels: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, name, slug, created_at, headline, subheadline, cta_label, thank_you)| {
                json!({
                    "id": id.to_string(),
                    "name": name,
                    "slug": slug,
                    "created_at": created_at,
                    // What the shared page says, so the editor opens on the CUSTOMER's words rather
                    // than empty boxes. NULL means "not set" and the renderer falls back.
                    "headline": headline,
                    "subheadline": subheadline,
                    "cta_label": cta_label,
                    "thank_you": thank_you,
                    // The page a customer shares: slug-only, no tenant in the path (see render_funnel).
                    "public_path": format!("/funnel/{slug}"),
                })
            },
        )
        .collect();
    Ok(Json(json!(funnels)))
}
pub async fn create_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    // The name is customer text that reaches an HTML <title> and a public URL, so it is normalised
    // (trimmed, non-empty, no control characters) BEFORE the slug is derived from it.
    let name = crate::security::display_name::normalize_capped(
        payload["name"].as_str().unwrap_or("New Funnel"),
        255,
        "funnel name",
    )
    .map_err(AppError::BadRequest)?;
    // `funnels.tenant_id` REFERENCES `tenants(id)`. The reads above may take the nil-UUID fallback
    // (they return no rows), but a CREATE must not: an admin session whose workspace row is gone is
    // exempt from `global_auth`'s existence gate, so the INSERT used to answer the foreign-key
    // violation as `500 {"error":"Database error"}` and write nothing — measured live 2026-10-15
    // (`POST /api/v1/funnels` on an admin token for a deleted workspace: 500; the same request on a
    // `user` token: 403 "Workspace no longer exists"). Refuse before the write.
    let tenant_id =
        crate::handlers::tenant_handler::require_existing_workspace(&state.pool, &auth.tenant_id)
            .await?;
    // The public page is addressed by SLUG ALONE (`/funnel/<slug>`, no tenant in the path), so the
    // slug must be (a) URL-safe and (b) unique across the fleet: the old transform was
    // `name.to_lowercase().replace(' ', "-")`, which put an apostrophe ("Sam's Funnel"),
    // or a SLASH ("Q3 / Q4 Promo" -> `q3-/-q4-promo`, a path that splits and does not route) into a
    // live URL, and let two workspaces that both called a funnel "My Funnel" share one page — one
    // workspace's name answered the other's link. `slugify` is the app's ONE slug transform; a name
    // of only punctuation slugifies to "", so that falls back to a stable word. A collision gets
    // `-2`, `-3`, … and `uq_funnels_slug` (migration 108) is what makes the insert refuse a race.
    let base = {
        let s = slugify(&name);
        if s.is_empty() {
            "funnel".to_string()
        } else {
            s
        }
    };
    let mut slug = base.clone();
    let mut attempt = 1u32;
    loop {
        let res = sqlx::query(
            "INSERT INTO funnels (id, tenant_id, name, slug) VALUES ($1, $2, $3, $4)
             ON CONFLICT (slug) DO NOTHING",
        )
        .bind(id)
        .bind(tenant_id)
        .bind(&name)
        .bind(&slug)
        .execute(&state.pool)
        .await?;
        if res.rows_affected() == 1 {
            break;
        }
        attempt += 1;
        if attempt > 50 {
            return Err(AppError::BadRequest(
                "Could not allocate a public link for that name: try a different name".into(),
            ));
        }
        slug = format!("{base}-{attempt}");
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id.to_string(),
            "name": name,
            "slug": slug,
            "public_path": format!("/funnel/{slug}"),
            "message": "Funnel created"
        })),
    ))
}
pub async fn get_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let row: Option<(Uuid, String, String, chrono::NaiveDateTime)> =
        sqlx::query_as("SELECT id, name, COALESCE(slug,''), created_at FROM funnels WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await?;
    let r = row.ok_or_else(|| AppError::NotFound("Funnel not found".into()))?;
    Ok(Json(
        json!({"id": r.0.to_string(), "name": r.1, "slug": r.2}),
    ))
}
pub async fn update_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    // A body that carries no writable field can never write, so it is refused BEFORE the lookup:
    // the answer is then identical for a real id and a random uuid, and this route cannot become an
    // existence oracle (kanban t_c2259356).
    if payload.get("name").is_none()
        && !PAGE_FIELDS
            .iter()
            .any(|(k, _, _)| payload.get(*k).is_some())
    {
        return Err(AppError::BadRequest(
            "No writable field: send 'name', 'headline', 'subheadline', 'cta_label' or 'thank_you'"
                .into(),
        ));
    }

    let existing: Option<(
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT name, headline, subheadline, cta_label, thank_you FROM funnels WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((mut name, mut headline, mut subheadline, mut cta_label, mut thank_you)) = existing
    else {
        return Err(AppError::NotFound("Funnel not found".into()));
    };

    // The NAME is the row's label and the page's <title>. The SLUG is the funnel's public address
    // and a customer may have shared (or printed) it, so a rename deliberately does NOT move the
    // link.
    if let Some(raw) = payload.get("name") {
        let Some(text) = raw.as_str() else {
            return Err(AppError::BadRequest("funnel name must be a string".into()));
        };
        name = crate::security::display_name::normalize_capped(text, 255, "funnel name")
            .map_err(AppError::BadRequest)?;
    }

    for (key, max_chars, label) in PAGE_FIELDS {
        let edit = field_edit(&payload, key, max_chars, label)?;
        let slot = match key {
            "headline" => &mut headline,
            "subheadline" => &mut subheadline,
            "cta_label" => &mut cta_label,
            _ => &mut thank_you,
        };
        match edit {
            FieldEdit::Absent => {}
            FieldEdit::Clear => *slot = None,
            FieldEdit::Set(v) => *slot = Some(v),
        }
    }

    sqlx::query(
        "UPDATE funnels SET name=$1, headline=$2, subheadline=$3, cta_label=$4, thank_you=$5 \
         WHERE id=$6 AND tenant_id=$7",
    )
    .bind(&name)
    .bind(&headline)
    .bind(&subheadline)
    .bind(&cta_label)
    .bind(&thank_you)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(json!({
        "message": "Funnel updated",
        "name": name,
        "headline": headline,
        "subheadline": subheadline,
        "cta_label": cta_label,
        "thank_you": thank_you,
    })))
}
pub async fn delete_funnel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = tenant_of(&auth);
    let result = sqlx::query("DELETE FROM funnels WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Funnel not found".into()));
    }
    Ok(Json(json!({"message": "Funnel deleted"})))
}
pub async fn render_funnel(
    axum::extract::Path(slug): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> axum::response::Response {
    // Load SEO settings for injection
    let seo_rows: Vec<(String, Value)> = sqlx::query_as::<_, (String, String)>(
        // site_settings.value is NULLABLE with no DEFAULT and this tuple's element is a plain
        // String, so a single NULL row broke the whole render (silently: unwrap_or_default).
        // kanban t_98f5292f.
        "SELECT COALESCE(key, ''), COALESCE(value::text, '') FROM site_settings WHERE key LIKE 'seo_%'",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| tracing::error!("render_funnel seo settings query failed: {:?}", e))
    .unwrap_or_default()
    .into_iter()
    .map(|(k, raw)| {
        (
            k,
            crate::handlers::site_settings_handler::value_from_text(&raw),
        )
    })
    .collect();
    let mut seo_meta = String::from("<meta name=\"description\" content=\"FunnelSwift — interactive funnel for leads and conversion.\">\n<meta property=\"og:type\" content=\"website\">\n<meta property=\"twitter:card\" content=\"summary_large_image\">\n");
    let mut seo_scripts = String::new();
    for (k, v) in &seo_rows {
        let short = k.strip_prefix("seo_").unwrap_or(k);
        match short {
            "google_analytics" => {
                if let Some(s) = v.as_str() {
                    seo_scripts.push_str(&format!("<script async src=\"https://www.googletagmanager.com/gtag/js?id={}\"></script><script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{}');</script>\n", s, s));
                }
            }
            "facebook_pixel" => {
                if let Some(s) = v.as_str() {
                    seo_scripts.push_str(&format!("<script>!function(f,b,e,v,n,t,s){{if(f.fbq)return;n=f.fbq=function(){{n.callMethod?n.callMethod.apply(n,arguments):n.queue.push(arguments)}};if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';n.queue=[];t=b.createElement(e);t.async=!0;t.src=v;s=b.getElementsByTagName(e)[0];s.parentNode.insertBefore(t,s)}}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','{}');fbq('track','PageView');</script><noscript><img height=\"1\" width=\"1\" src=\"https://www.facebook.com/tr?id={}&ev=PageView&noscript=1\"/></noscript>\n", s, s));
                }
            }
            _ => {}
        }
    }
    seo_meta.push_str(&format!(
        "<link rel=\"canonical\" href=\"https://funnelswift.net/funnel/{}\">\n",
        html_escape(&slug)
    ));
    seo_meta.push_str("<link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">\n<link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\n");

    // Try to get the funnel row. The row decides the STATUS (kanban t_80649ea9): no row at all
    // is a missing resource -> 404, a failed lookup is a server error -> 500, a real row -> 200.
    // 410 is deliberately not used: a funnel slug can be created again, so "gone for good" would
    // be a lie.
    let looked_up = sqlx::query_as::<_, (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>(
        "SELECT name, headline, subheadline, cta_label, thank_you FROM funnels WHERE slug = $1 ORDER BY created_at ASC LIMIT 1",
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| tracing::error!("render_funnel lookup failed for slug '{}': {:?}", slug, e));

    let (status, row) = match looked_up {
        Ok(Some(r)) => (StatusCode::OK, Some(r)),
        Ok(None) => (StatusCode::NOT_FOUND, None),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, None),
    };

    // The CUSTOMER's words where they set them, a sensible default where they did not: every row
    // that existed before migration 111 keeps rendering, and it now renders a page that can take a
    // lead instead of "under construction". The RAW values are kept for the inline script (a JS
    // string must not carry HTML entities) and escaped only on the way into the markup.
    let (raw_name, raw_headline, raw_subheadline, raw_cta, raw_thanks) = match row {
        Some((name, h, sl, c, t)) => {
            let headline = h
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| name.clone());
            let subheadline = sl
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "Leave your details and we'll be in touch.".to_string());
            let cta_label = c
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "Get started".to_string());
            let thank_you = t
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "Thanks — you're on the list. We'll be in touch.".to_string());
            (name, headline, subheadline, cta_label, thank_you)
        }
        None => (
            slug.clone(),
            "This page is not available.".to_string(),
            "The link may have been mistyped, or the page was removed.".to_string(),
            "Get started".to_string(),
            "Thanks — you're on the list. We'll be in touch.".to_string(),
        ),
    };
    let (funnel_name, headline, subheadline, cta_label) = (
        html_escape(&raw_name),
        html_escape(&raw_headline),
        html_escape(&raw_subheadline),
        html_escape(&raw_cta),
    );

    (
        status,
        axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{funnel_name} — FunnelSwift</title>
{seo_meta}
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:'Plus Jakarta Sans','Inter',system-ui,-apple-system,sans-serif;background:linear-gradient(135deg,#1e1b4b 0%,#312e81 100%);color:#fff;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:24px}}
.card{{width:100%;max-width:460px;background:rgba(255,255,255,.06);border:1px solid rgba(255,255,255,.14);border-radius:16px;padding:32px}}
h1{{font-size:30px;line-height:1.22;margin-bottom:10px}}
p.sub{{color:#c7d2fe;font-size:16px;margin-bottom:22px}}
label{{display:block;font-size:12px;letter-spacing:.04em;text-transform:uppercase;color:#a5b4fc;margin:12px 0 6px}}
input{{width:100%;padding:12px 14px;border-radius:9px;border:1px solid rgba(255,255,255,.18);background:rgba(15,23,42,.55);color:#fff;font-size:15px}}
input:focus{{outline:2px solid #818cf8;border-color:transparent}}
button{{width:100%;margin-top:18px;padding:13px 16px;border:0;border-radius:9px;background:#6366f1;color:#fff;font-size:15px;font-weight:600;cursor:pointer}}
button:disabled{{opacity:.6;cursor:default}}
.err{{color:#fca5a5;font-size:13px;margin-top:10px;min-height:16px}}
</style>
{seo_scripts}
</head>
<body data-slug="{slug_attr}">
<div class="card" id="fn-card">
<h1>{headline}</h1>
<p class="sub">{subheadline}</p>
<form id="fn-form" novalidate>
<label for="fn-name-input">Your name</label>
<input id="fn-name-input" name="name" autocomplete="name" maxlength="120" placeholder="Alex Doe">
<label for="fn-email-input">Email</label>
<input id="fn-email-input" name="email" type="email" autocomplete="email" maxlength="200" placeholder="alex@example.com" required>
<button id="fn-send" type="submit">{cta_label}</button>
<div class="err" id="fn-err"></div>
</form>
</div>
<script>
(function(){{
  var slug = document.body.getAttribute('data-slug');
  var form = document.getElementById('fn-form');
  var send = document.getElementById('fn-send');
  var err  = document.getElementById('fn-err');
  var thanks   = {thank_you_js};
  var ctaLabel = {cta_js};
  if(!form) return;
  form.addEventListener('submit', function(ev){{
    ev.preventDefault();
    var name  = document.getElementById('fn-name-input').value.trim();
    var email = document.getElementById('fn-email-input').value.trim();
    err.textContent = '';
    if(!email || email.indexOf('@') < 1){{ err.textContent = 'Please enter your email address.'; return; }}
    send.disabled = true;
    send.textContent = 'Sending…';
    fetch('/funnel/' + encodeURIComponent(slug) + '/submit', {{
      method: 'POST',
      headers: {{ 'Content-Type': 'application/json' }},
      body: JSON.stringify({{ name: name, email: email }})
    }}).then(function(r){{
      return r.json().then(function(j){{ return {{ ok: r.ok, j: j }}; }});
    }}).then(function(o){{
      if(!o.ok) throw new Error((o.j && (o.j.message || o.j.error)) || 'Could not send that.');
      document.getElementById('fn-card').innerHTML = '<h1>' + thanks + '</h1>';
    }}).catch(function(e){{
      send.disabled = false;
      send.textContent = ctaLabel;
      err.textContent = e.message || 'Could not send that.';
    }});
  }});
}})();
</script>
</body>
</html>"#,
        funnel_name = funnel_name,
        seo_meta = seo_meta,
        seo_scripts = seo_scripts,
        slug_attr = html_escape(&slug),
        headline = headline,
        subheadline = subheadline,
        cta_label = cta_label,
        thank_you_js = js_string(&raw_thanks),
        cta_js = js_string(&raw_cta),
        )),
    )
        .into_response()
}

/// `POST /funnel/:slug/submit` — the public opt-in form on a shared funnel page.
///
/// The visitor is anonymous by construction: no session, no cookie, no credential anywhere in the
/// body. The SLUG resolves the tenant, so the write is scoped by the row itself and nothing a
/// stranger sends can name another workspace. The lead lands in the same `leads` table the panel
/// reads, with `source = 'funnel'` and the funnel's own id in `custom_fields`, so the customer can
/// see WHICH page produced it. A missing slug is 404 (not 403: a refusal must not confirm a page
/// exists either).
pub async fn submit_funnel_lead(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let funnel: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, tenant_id FROM funnels WHERE slug = $1 LIMIT 1")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await?;
    let Some((funnel_id, tenant_id)) = funnel else {
        return Err(AppError::NotFound("Funnel not found".into()));
    };

    // The EMAIL is the point of the form — the page's own field is `required`, so the server must
    // not accept a submission the browser would have refused: a lead with no address cannot be
    // followed up, and storing one would make the panel's funnel count a lie. The name is optional
    // (a visitor who types only an address still becomes a usable lead, named from that address).
    let Some(raw_email) = public_field(&body, "email") else {
        return Err(AppError::Validation(
            "Please enter your email address.".into(),
        ));
    };
    // A visitor typed this address, so it is validated BEFORE the write with the same normaliser
    // the panel's create uses: a lead nobody could ever mail is worse than a refusal.
    let email = crate::security::email_addr::normalize(&raw_email).map_err(AppError::Validation)?;
    let name = public_field(&body, "name").unwrap_or_else(|| {
        email
            .split('@')
            .next()
            .filter(|p| !p.is_empty())
            .unwrap_or("Funnel visitor")
            .to_string()
    });
    // The plan's lead ceiling binds this public door too — same helper, same key and same 402
    // sentence as the panel's create and the Kinetic card's form.
    crate::features::enforce_feature_limit(&state, tenant_id, "max_leads", "Leads").await?;

    let name = crate::security::lead_fields::fit(&name, crate::security::lead_fields::TEXT_CHARS);
    let email = crate::security::lead_fields::fit_opt(
        Some(email.as_str()),
        crate::security::lead_fields::TEXT_CHARS,
    );
    let custom_fields = json!({
        "funnel_id": funnel_id.to_string(),
        "funnel_slug": slug,
    });

    let lead_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO leads (id, tenant_id, name, email, source, status, custom_fields)
           VALUES ($1, $2, $3, $4, 'funnel', 'new', $5)"#,
    )
    .bind(lead_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(&email)
    .bind(&custom_fields)
    .execute(&state.pool)
    .await?;

    // A captured lead is a captured lead, whichever door it came through: the same `lead.created`
    // event every other writer of a `leads` row emits.
    crate::webhooks::spawn(
        state.pool.clone(),
        tenant_id,
        crate::webhooks::LEAD_CREATED,
        crate::webhooks::lead_created(
            lead_id,
            Some(name.as_str()),
            email.as_deref(),
            None,
            None,
            Some("funnel"),
            &[],
        ),
    );

    Ok((StatusCode::CREATED, Json(json!({ "status": "ok" }))))
}

/// The console may write these four strings to a funnel's public page; the tuple is
/// (json key, character cap, the name a refusal shows the customer). The columns are `text`, so the
/// caps are DISPLAY widths, not storage: a pasted paragraph is refused with the field's own name
/// instead of being silently truncated. `normalize_capped` also refuses control characters, which
/// would otherwise reach the page.
const PAGE_FIELDS: [(&str, usize, &str); 4] = [
    ("headline", 200, "headline"),
    ("subheadline", 400, "subheadline"),
    ("cta_label", 60, "button label"),
    ("thank_you", 300, "thank-you message"),
];

/// One optional page field as it arrived: ABSENT (leave the stored value alone), CLEAR (the
/// customer emptied the box — store NULL and fall back to the renderer's default), or SET.
enum FieldEdit {
    Absent,
    Clear,
    Set(String),
}

fn field_edit(payload: &Value, key: &str, max_chars: usize, label: &str) -> AppResult<FieldEdit> {
    let Some(raw) = payload.get(key) else {
        return Ok(FieldEdit::Absent);
    };
    let Some(text) = raw.as_str() else {
        return Err(AppError::BadRequest(format!("{label} must be a string")));
    };
    if text.trim().is_empty() {
        return Ok(FieldEdit::Clear);
    }
    crate::security::display_name::normalize_capped(text, max_chars, label)
        .map(FieldEdit::Set)
        .map_err(AppError::BadRequest)
}

fn public_field(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A Rust string as a JavaScript string LITERAL, safe inside an inline `<script>`: JSON quoting
/// plus the `</` escape, so a customer's thank-you message can never close the script tag.
fn js_string(raw: &str) -> String {
    serde_json::to_string(raw)
        .unwrap_or_else(|_| "\"\"".to_string())
        .replace("</", "<\\/")
}
