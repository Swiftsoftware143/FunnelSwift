use axum::{extract::{Path, Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CardQuery { pub page: Option<i32>, pub per_page: Option<i32>, pub type_: Option<String> }

pub async fn list_cards(auth: AuthUser, State(state): State<AppState>, Query(q): Query<CardQuery>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let rows = sqlx::query(
        "SELECT id, tenant_id, title, slug, bio, bg_color, accent_color, text_color, sub_color, btn_color, card_type, tagline, meta_description, avatar_url, social_links, theme_slug, video_provider, video_id, is_template, template_category, category, cta_text, created_at, updated_at FROM kinetic_cards WHERE tenant_id = $1 ORDER BY created_at DESC"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    use sqlx::Row;
    let cards: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").unwrap_or_default().to_string(),
        "title": r.try_get::<String, _>("title").unwrap_or_default(),
        "slug": r.try_get::<String, _>("slug").unwrap_or_default(),
        "bio": r.try_get::<Option<String>, _>("bio").unwrap_or_default(),
        "bg_color": r.try_get::<String, _>("bg_color").unwrap_or_default(),
        "accent_color": r.try_get::<String, _>("accent_color").unwrap_or_default(),
        "text_color": r.try_get::<String, _>("text_color").unwrap_or_default(),
        "sub_color": r.try_get::<String, _>("sub_color").unwrap_or_default(),
        "btn_color": r.try_get::<String, _>("btn_color").unwrap_or_default(),
        "card_type": r.try_get::<String, _>("card_type").unwrap_or_default(),
        "tagline": r.try_get::<Option<String>, _>("tagline").unwrap_or_default(),
        "meta_description": r.try_get::<Option<String>, _>("meta_description").unwrap_or_default(),
        "avatar_url": r.try_get::<Option<String>, _>("avatar_url").unwrap_or_default(),
        "social_links": r.try_get::<Option<Value>, _>("social_links").unwrap_or_default(),
        "theme_slug": r.try_get::<Option<String>, _>("theme_slug").unwrap_or_default(),
        "video_provider": r.try_get::<Option<String>, _>("video_provider").unwrap_or_default(),
        "video_id": r.try_get::<Option<String>, _>("video_id").unwrap_or_default(),
        "is_template": r.try_get::<bool, _>("is_template").unwrap_or(false),
        "template_category": r.try_get::<Option<String>, _>("template_category").unwrap_or_default(),
        "category": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "cta_text": r.try_get::<Option<String>, _>("cta_text").unwrap_or_default(),
        "created_at": r.try_get::<chrono::NaiveDateTime, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::NaiveDateTime, _>("updated_at").unwrap_or_default()
    })).collect();
    Ok(Json(json!({"cards": cards})))
}

pub async fn create_card(auth: AuthUser, State(state): State<AppState>, Json(body): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let id = Uuid::new_v4();
    let title = body["title"].as_str().unwrap_or("New Card");
    let id_str = id.to_string();
    let slug = body["slug"].as_str().unwrap_or(&id_str[..8]);
    let bio = body["bio"].as_str().unwrap_or("");
    let bg_color = body["bg_color"].as_str().unwrap_or("#0f172a");
    let accent_color = body["accent_color"].as_str().unwrap_or("#6366f1");
    let text_color = body["text_color"].as_str().unwrap_or("#ffffff");
    let sub_color = body["sub_color"].as_str().unwrap_or("#94a3b8");
    let btn_color = body["btn_color"].as_str().unwrap_or("#6366f1");
    let card_type = body["type"].as_str().or(body["card_type"].as_str()).unwrap_or("bio-link");
    let tagline = body["tagline"].as_str().unwrap_or("");
    let meta_desc = body["meta_description"].as_str().unwrap_or("");
    let avatar = body["avatar_url"].as_str();
    let social = body.get("social_links").cloned();
    let theme = body["theme_slug"].as_str();
    let video_provider = body["video_provider"].as_str();
    let video_id = body["video_id"].as_str();
    let is_template = body["is_template"].as_bool().unwrap_or(false);
    let template_category = body["template_category"].as_str();
    let category = body["category"].as_str();
    let cta_text = body["cta_text"].as_str();

    sqlx::query("INSERT INTO kinetic_cards (id, tenant_id, title, slug, bio, bg_color, accent_color, text_color, sub_color, btn_color, card_type, tagline, meta_description, avatar_url, social_links, theme_slug, video_provider, video_id, is_template, template_category, category, cta_text) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)")
        .bind(id).bind(tenant_id).bind(title).bind(slug).bind(bio).bind(bg_color).bind(accent_color).bind(text_color).bind(sub_color).bind(btn_color).bind(card_type).bind(tagline).bind(meta_desc).bind(avatar).bind(&social).bind(theme).bind(video_provider).bind(video_id).bind(is_template).bind(template_category).bind(category).bind(cta_text)
        .execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "slug": slug, "message": "Card created"}))))
}

pub async fn update_card(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let social = body.get("social_links").cloned();
    sqlx::query("UPDATE kinetic_cards SET title=COALESCE($3,title), bio=COALESCE($4,bio), bg_color=COALESCE($5,bg_color), accent_color=COALESCE($6,accent_color), text_color=COALESCE($7,text_color), sub_color=COALESCE($8,sub_color), btn_color=COALESCE($9,btn_color), avatar_url=COALESCE($10,avatar_url), social_links=COALESCE($11,social_links), tagline=COALESCE($12,tagline), meta_description=COALESCE($13,meta_description), video_provider=COALESCE($14,video_provider), video_id=COALESCE($15,video_id), cta_text=COALESCE($16,cta_text), slug=COALESCE($17,slug) WHERE id=$1 AND tenant_id=$2")
        .bind(id).bind(tenant_id)
        .bind(body["title"].as_str()).bind(body["bio"].as_str())
        .bind(body["bg_color"].as_str()).bind(body["accent_color"].as_str())
        .bind(body["text_color"].as_str()).bind(body["sub_color"].as_str())
        .bind(body["btn_color"].as_str()).bind(body["avatar_url"].as_str())
        .bind(&social)
        .bind(body["tagline"].as_str()).bind(body["meta_description"].as_str())
        .bind(body["video_provider"].as_str()).bind(body["video_id"].as_str())
        .bind(body["cta_text"].as_str()).bind(body["slug"].as_str())
        .execute(&state.pool).await?;
    Ok(Json(json!({"message": "Card updated"})))
}

pub async fn delete_card(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    sqlx::query("DELETE FROM kinetic_cards WHERE id=$1 AND tenant_id=$2").bind(id).bind(tenant_id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Card deleted"})))
}

pub async fn list_buttons(_auth: AuthUser, State(state): State<AppState>, Path(card_id): Path<Uuid>) -> AppResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT id, card_id, label, url, sort_order, created_at FROM kinetic_buttons WHERE card_id=$1 ORDER BY sort_order"
    ).bind(card_id).fetch_all(&state.pool).await.unwrap_or_default();
    use sqlx::Row;
    let buttons: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").unwrap_or_default().to_string(),
        "card_id": r.try_get::<Uuid, _>("card_id").unwrap_or_default().to_string(),
        "label": r.try_get::<String, _>("label").unwrap_or_default(),
        "url": r.try_get::<String, _>("url").unwrap_or_default(),
        "sort_order": r.try_get::<i32, _>("sort_order").unwrap_or(0),
        "created_at": r.try_get::<chrono::NaiveDateTime, _>("created_at").unwrap_or_default()
    })).collect();
    Ok(Json(json!(buttons)))
}

pub async fn create_button(_auth: AuthUser, State(state): State<AppState>, Path(card_id): Path<Uuid>, Json(body): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let label = body["label"].as_str().unwrap_or("Button");
    let url = body["url"].as_str().unwrap_or("");
    let sort = body["sort_order"].as_i64().unwrap_or(0) as i32;
    sqlx::query("INSERT INTO kinetic_buttons (id, card_id, label, url, sort_order) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(card_id).bind(label).bind(url).bind(sort).execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn delete_button(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM kinetic_buttons WHERE id=$1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Deleted"})))
}

pub async fn list_sources(_auth: AuthUser, State(_state): State<AppState>, Path(_card_id): Path<Uuid>) -> AppResult<Json<Value>> {
    Ok(Json(json!([])))
}
pub async fn create_source(_auth: AuthUser, State(_state): State<AppState>, Path(_card_id): Path<Uuid>, Json(_body): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(json!({"id": Uuid::new_v4()}))))
}
pub async fn delete_source(_auth: AuthUser, State(_state): State<AppState>, Path(_id): Path<Uuid>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "Deleted"})))
}

pub async fn get_metrics(_auth: AuthUser, State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({"views":0,"clicks":0,"leads":0})))
}

pub async fn get_subdomain(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let row = sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE tenant_id=$1 AND key='subdomain'").bind(tenant_id).fetch_optional(&state.pool).await?;
    Ok(Json(json!({"subdomain": row.map(|r| r.0).unwrap_or_default()})))
}
pub async fn set_subdomain(auth: AuthUser, State(state): State<AppState>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let val = body["subdomain"].as_str().unwrap_or("ss");
    sqlx::query("INSERT INTO settings (id, tenant_id, key, value) VALUES ($1,$2,'subdomain',$3) ON CONFLICT (tenant_id, key) DO UPDATE SET value=$3")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(val).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Saved"})))
}
pub async fn get_custom_domain(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let row = sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE tenant_id=$1 AND key='custom_domain'").bind(tenant_id).fetch_optional(&state.pool).await?;
    Ok(Json(json!({"custom_domain": row.map(|r| r.0).unwrap_or_default()})))
}
pub async fn set_custom_domain(auth: AuthUser, State(state): State<AppState>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth.tenant_id.parse().map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let val = body["custom_domain"].as_str().unwrap_or("");
    sqlx::query("INSERT INTO settings (id, tenant_id, key, value) VALUES ($1,$2,'custom_domain',$3) ON CONFLICT (tenant_id, key) DO UPDATE SET value=$3")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(val).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Saved"})))
}

pub async fn render_card(axum::extract::Path(slug): axum::extract::Path<String>, State(state): State<AppState>) -> impl axum::response::IntoResponse {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT id, tenant_id, title, slug, bio, bg_color, accent_color, text_color, sub_color, btn_color, card_type, tagline, meta_description, avatar_url, social_links, theme_slug, video_provider, video_id, is_template, template_category, category, cta_text, created_at, updated_at FROM kinetic_cards WHERE slug = $1 LIMIT 1"
    ).bind(&slug).fetch_optional(&state.pool).await.unwrap_or(None);

    if row.is_none() {
        return axum::response::Html("<html><body style='background:#0f172a;color:#fff;display:flex;align-items:center;justify-content:center;height:100vh;font-family:sans-serif'><div style='text-align:center'><h1 style='font-size:48px;margin-bottom:8px'>404</h1><p>Card not found</p><a href='https://funnelswift.net/kinetic' style='color:#a855f7'>Create your own →</a></div></body></html>".to_string());
    }

    let r = row.unwrap();
    let card_type: String = r.try_get("card_type").unwrap_or_default();
    let title: String = r.try_get("title").unwrap_or_default();
    let bio: String = r.try_get("bio").unwrap_or_default();
    let bg: String = r.try_get("bg_color").unwrap_or_default();
    let accent: String = r.try_get("accent_color").unwrap_or_default();
    let text: String = r.try_get("text_color").unwrap_or_default();
    let avatar: Option<String> = r.try_get("avatar_url").unwrap_or(None);
    let tagline: Option<String> = r.try_get("tagline").unwrap_or(None);
    let meta_desc: Option<String> = r.try_get("meta_description").unwrap_or(None);
    let social_links: Option<Value> = r.try_get("social_links").unwrap_or(None);
    let cta_text: Option<String> = r.try_get("cta_text").unwrap_or(None);
    let video_provider: Option<String> = r.try_get("video_provider").unwrap_or(None);
    let video_id: Option<String> = r.try_get("video_id").unwrap_or(None);

    let av_html = if let Some(ref a) = avatar { if a.is_empty() { String::new() } else { format!("<img src='{}' class='av' alt='' onerror=\"this.style.display='none'\">", a) } } else { String::new() };
    let tag_html = if let Some(ref t) = tagline { if t.is_empty() { String::new() } else { format!("<p class='tag'>{}</p>", t) } } else { String::new() };
    let bio_html = if !bio.is_empty() { format!("<p class='bio'>{}</p>", bio) } else { String::new() };
    let cta_html = if let Some(ref c) = cta_text { if c.is_empty() { String::new() } else { format!("<a href='#' class='cta'>{}</a>", c) } } else { String::new() };

    let bg_gradient = format!("radial-gradient(circle at 50% 25%, {}44 0%, {} 70%)", accent, bg);

    let social_html = if let Some(ref sl) = social_links {
        let mut s = String::from("<div class='socials'>");
        if let Some(arr) = sl.as_array() {
            for item in arr {
                let platform = item["platform"].as_str().unwrap_or("link");
                let url = item["url"].as_str().unwrap_or("#");
                s.push_str(&format!("<a href='{}' class='s-icon' target='_blank' rel='noopener'>{}</a>", url, platform));
            }
        }
        s.push_str("</div>");
        s
    } else { String::new() };

    let page_title = if meta_desc.as_ref().map_or(true, |m| m.is_empty()) { title.clone() } else { format!("{} — {}", title, meta_desc.as_ref().unwrap()) };

    axum::response::Html(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{page_title}</title>
<meta name="description" content="{meta}">
<meta property="og:title" content="{page_title}">
<meta property="og:description" content="{meta}">
<meta property="og:type" content="website">
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:'Plus Jakarta Sans','Inter',system-ui,-apple-system,sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;padding:24px 16px;color:{text};overflow-x:hidden}}
.bg{{position:fixed;inset:0;z-index:-1;background:{bg_gradient}}}
.noise{{position:fixed;inset:0;z-index:0;opacity:.035;background-image:url("data:image/svg+xml,%3Csvg viewBox='0 0 256 256' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.8' numOctaves='4' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");pointer-events:none}}
.glow{{position:fixed;top:40%;left:50%;transform:translate(-50%,-50%);width:200px;height:200px;border-radius:50%;background:radial-gradient(circle,{accent}44 0%,transparent 70%);pointer-events:none;z-index:0}}
.card{{position:relative;z-index:2;max-width:360px;width:100%;text-align:center;display:flex;flex-direction:column;align-items:center;gap:12px;animation:fadeIn .6s ease-out}}
.av{{width:80px;height:80px;border-radius:50%;object-fit:cover;box-shadow:0 0 0 4px {accent},0 0 24px {accent}66;animation:pulse-glow 2.5s ease-in-out infinite}}
h1{{font-size:26px;font-weight:800;text-shadow:0 2px 8px rgba(0,0,0,.3)}}
.tag{{font-size:14px;color:{accent};font-weight:600;letter-spacing:.5px}}
.bio{{font-size:14px;line-height:1.6;opacity:.85;max-width:300px}}
.cta{{display:inline-block;padding:14px 36px;background:{accent};color:{text};border-radius:14px;font-size:15px;font-weight:700;text-decoration:none;box-shadow:0 4px 18px {accent}44;transition:all .2s;margin-top:4px}}
.cta:hover{{transform:translateY(-2px);box-shadow:0 6px 24px {accent}66}}
.socials{{display:flex;gap:10px;flex-wrap:wrap;justify-content:center;margin-top:4px}}
.s-icon{{display:inline-flex;align-items:center;gap:4px;padding:7px 16px;background:rgba(255,255,255,.08);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px);border:1px solid rgba(255,255,255,.12);border-radius:20px;font-size:12px;color:{text};text-decoration:none;transition:all .2s}}
.s-icon:hover{{background:rgba(255,255,255,.15);border-color:rgba(255,255,255,.25)}}
.badge{{position:absolute;top:16px;right:16px;font-size:10px;padding:4px 10px;border-radius:100px;background:rgba(255,255,255,.08);backdrop-filter:blur(8px);border:1px solid rgba(255,255,255,.1);color:{text};opacity:.6;text-decoration:none;z-index:10}}
.badge:hover{{opacity:1}}
@keyframes fadeIn{{from{{opacity:0;transform:translateY(16px)}}to{{opacity:1;transform:translateY(0)}}}}
@keyframes pulse-glow{{0%,100%{{box-shadow:0 0 0 4px {accent},0 0 24px {accent}66}}50%{{box-shadow:0 0 0 5px {accent},0 0 36px {accent}88}}}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<div class="bg"></div>
<div class="noise"></div>
<div class="glow"></div>
<a href="https://funnelswift.net/kinetic" class="badge">Create your card →</a>
<div class="card">
{av_html}
<h1>{title}</h1>
{tag_html}
{bio_html}
{cta_html}
{social_html}
</div>
</body>
</html>"#, 
        page_title = page_title, meta = meta_desc.unwrap_or_default(), text = text, 
        bg_gradient = bg_gradient, accent = accent, title = title,
        av_html = av_html, tag_html = tag_html, bio_html = bio_html, cta_html = cta_html, social_html = social_html
    ))
}
pub async fn submit_lead(axum::extract::Path(slug): axum::extract::Path<String>, State(_state): State<AppState>, Json(_body): Json<Value>) -> Json<Value> {
    Json(json!({"message": "Lead submitted", "slug": slug}))
}
pub async fn track_click(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({}))
}
