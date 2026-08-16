use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

/// Resolve canonical URL from Host header
/// kntcrd.com subdomain → https://{tenant}.kntcrd.com/k/{slug}
/// funnelswift.net → 301 redirect to kntcrd.com (never canonical)
/// custom domain → https://{domain}/k/{slug}
/// Resolve canonical URL from Host header with correct prefix
/// kntcrd.com subdomain → https://{tenant}.kntcrd.com/{prefix}/{slug}
/// funnelswift.net → redirect to kntcrd.com root
/// custom domain → https://{domain}/{prefix}/{slug}
fn resolve_canonical_url(host: &str, prefix: &str, slug: &str) -> String {
    let host_clean = host.split(':').next().unwrap_or(host).to_lowercase();
    if host_clean.ends_with("funnelswift.net") {
        return format!("https://kntcrd.com/{}/{}", prefix, slug);
    }
    if host_clean.ends_with("kntcrd.com") {
        return format!("https://{}/{}/{}", host_clean, prefix, slug);
    }
    format!("https://{}/{}/{}", host_clean, prefix, slug)
}

/// Map URL prefix to branded CTA label + card type label
fn cta_for_prefix(prefix: &str) -> (&'static str, &'static str) {
    match prefix {
        "b" => ("Claim your free Bio Link →", "Bio Link"),
        "c" => (
            "Claim your free Digital Business Card →",
            "Digital Business Card",
        ),
        "m" => ("Claim your free Micro Page →", "Micro Page"),
        "f" => ("Claim your free Mini Funnel →", "Mini Funnel"),
        "h" => ("Claim your free Hero Page →", "Hero Page"),
        _ => ("Claim your free Kinetic Card →", "Kinetic Card"),
    }
}

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::templates::html_escape;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CardQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
    pub type_: Option<String>,
}

pub async fn list_cards(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(_q): Query<CardQuery>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let rows = sqlx::query(
        "SELECT id, tenant_id, user_id, title, slug, bio, bg_color, accent_color, text_color, button_bg_color, button_text_color, template_type, tagline, meta_description, avatar_url, layout_blocks, theme, video_provider, video_id, is_active, created_at, updated_at FROM kinetic_cards WHERE tenant_id = $1 ORDER BY created_at DESC"
    ).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
    use sqlx::Row;
    let cards: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").unwrap_or_default().to_string(),
        "title": r.try_get::<String, _>("title").unwrap_or_default(),
        "slug": r.try_get::<String, _>("slug").unwrap_or_default(),
        "bio": r.try_get::<Option<String>, _>("bio").unwrap_or_default(),
        "bg_color": r.try_get::<Option<String>, _>("bg_color").unwrap_or_default(),
        "accent_color": r.try_get::<Option<String>, _>("accent_color").unwrap_or_default(),
        "text_color": r.try_get::<Option<String>, _>("text_color").unwrap_or_default(),
        "button_bg_color": r.try_get::<Option<String>, _>("button_bg_color").unwrap_or_default(),
        "button_text_color": r.try_get::<Option<String>, _>("button_text_color").unwrap_or_default(),
        "template_type": r.try_get::<Option<String>, _>("template_type").unwrap_or_default(),
        "tagline": r.try_get::<Option<String>, _>("tagline").unwrap_or_default(),
        "meta_description": r.try_get::<Option<String>, _>("meta_description").unwrap_or_default(),
        "avatar_url": r.try_get::<Option<String>, _>("avatar_url").unwrap_or_default(),
        "layout_blocks": r.try_get::<Option<Value>, _>("layout_blocks").unwrap_or_default(),
        "theme": r.try_get::<Option<String>, _>("theme").unwrap_or_default(),
        "video_provider": r.try_get::<Option<String>, _>("video_provider").unwrap_or_default(),
        "video_id": r.try_get::<Option<String>, _>("video_id").unwrap_or_default(),
        "is_template": r.try_get::<bool, _>("is_template").unwrap_or(false),
        "template_category": r.try_get::<Option<String>, _>("template_category").unwrap_or_default(),
        "category": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "created_at": r.try_get::<chrono::NaiveDateTime, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::NaiveDateTime, _>("updated_at").unwrap_or_default()
    })).collect();
    Ok(Json(json!({"cards": cards})))
}

pub async fn create_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
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
    let card_type = body["type"]
        .as_str()
        .or(body["card_type"].as_str())
        .unwrap_or("bio-link");
    let tagline = body["tagline"].as_str().unwrap_or("");
    let meta_desc = body["meta_description"].as_str().unwrap_or("");
    let avatar = body["avatar_url"].as_str();
    let social = body.get("social_links").cloned();
    let theme = body["theme_slug"].as_str();
    let video_provider = body["video_provider"].as_str();
    let video_id = body["video_id"].as_str();

    sqlx::query("INSERT INTO kinetic_cards (id, tenant_id, user_id, title, slug, bio, bg_color, accent_color, text_color, button_bg_color, button_text_color, template_type, tagline, meta_description, avatar_url, layout_blocks, theme, video_provider, video_id, is_active) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,true)")
        .bind(id).bind(tenant_id).bind(id).bind(title).bind(slug).bind(bio).bind(bg_color).bind(accent_color).bind(text_color).bind(sub_color).bind(btn_color).bind(card_type).bind(tagline).bind(meta_desc).bind(avatar).bind(&social).bind(theme).bind(video_provider).bind(video_id)
        .execute(&state.pool).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "slug": slug, "message": "Card created"})),
    ))
}

pub async fn update_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let social = body.get("social_links").cloned();
    sqlx::query("UPDATE kinetic_cards SET title=COALESCE($3,title), bio=COALESCE($4,bio), bg_color=COALESCE($5,bg_color), accent_color=COALESCE($6,accent_color), text_color=COALESCE($7,text_color), button_bg_color=COALESCE($8,button_bg_color), button_text_color=COALESCE($9,button_text_color), avatar_url=COALESCE($10,avatar_url), layout_blocks=COALESCE($11,layout_blocks), tagline=COALESCE($12,tagline), meta_description=COALESCE($13,meta_description), video_provider=COALESCE($14,video_provider), video_id=COALESCE($15,video_id), cta_text=COALESCE($16,cta_text), slug=COALESCE($17,slug), theme=COALESCE($18,theme) WHERE id=$1 AND tenant_id=$2")
        .bind(id).bind(tenant_id)
        .bind(body["title"].as_str()).bind(body["bio"].as_str())
        .bind(body["bg_color"].as_str()).bind(body["accent_color"].as_str())
        .bind(body["text_color"].as_str()).bind(body["sub_color"].as_str())
        .bind(body["btn_color"].as_str()).bind(body["avatar_url"].as_str())
        .bind(&social)
        .bind(body["tagline"].as_str()).bind(body["meta_description"].as_str())
        .bind(body["video_provider"].as_str()).bind(body["video_id"].as_str())
        .bind(body["cta_text"].as_str()).bind(body["slug"].as_str())
        .bind(body["theme"].as_str())
        .execute(&state.pool).await?;
    Ok(Json(json!({"message": "Card updated"})))
}

pub async fn delete_card(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    sqlx::query("DELETE FROM kinetic_cards WHERE id=$1 AND tenant_id=$2")
        .bind(id)
        .bind(tenant_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"message": "Card deleted"})))
}

pub async fn list_buttons(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    let rows = sqlx::query(
        "SELECT b.id, b.card_id, b.label, b.url, b.sort_order, b.created_at FROM kinetic_buttons b JOIN kinetic_cards c ON c.id = b.card_id WHERE b.card_id=$1 AND c.tenant_id=$2 ORDER BY b.sort_order"
    ).bind(card_id).bind(tenant_id).fetch_all(&state.pool).await.unwrap_or_default();
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

pub async fn create_button(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(card_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    // Verify the card belongs to this tenant before attaching a button.
    let owns_card: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kinetic_cards WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(card_id)
    .bind(tenant_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(false);
    if !owns_card {
        return Err(AppError::NotFound("Card not found".into()));
    }
    let id = Uuid::new_v4();
    let label = body["label"].as_str().unwrap_or("Button");
    let url = body["url"].as_str().unwrap_or("");
    let sort = body["sort_order"].as_i64().unwrap_or(0) as i32;
    sqlx::query(
        "INSERT INTO kinetic_buttons (id, card_id, label, url, sort_order) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(id)
    .bind(card_id)
    .bind(label)
    .bind(url)
    .bind(sort)
    .execute(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn delete_button(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query(
        "DELETE FROM kinetic_buttons b USING kinetic_cards c WHERE b.id=$1 AND b.card_id = c.id AND c.tenant_id=$2",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"message": "Deleted"})))
}

pub async fn list_sources(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_card_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!([])))
}
pub async fn create_source(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_card_id): Path<Uuid>,
    Json(_body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(json!({"id": Uuid::new_v4()}))))
}
pub async fn delete_source(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"message": "Deleted"})))
}

pub async fn get_metrics(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!({"views":0,"clicks":0,"leads":0})))
}

pub async fn get_subdomain(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT value FROM settings WHERE tenant_id=$1 AND key='subdomain'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(
        json!({"subdomain": row.map(|r| r.0).unwrap_or_default()}),
    ))
}
pub async fn set_subdomain(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let val = body["subdomain"].as_str().unwrap_or("ss");
    sqlx::query("INSERT INTO settings (id, tenant_id, key, value) VALUES ($1,$2,'subdomain',$3) ON CONFLICT (tenant_id, key) DO UPDATE SET value=$3")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(val).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Saved"})))
}
pub async fn get_custom_domain(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT value FROM settings WHERE tenant_id=$1 AND key='custom_domain'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(
        json!({"custom_domain": row.map(|r| r.0).unwrap_or_default()}),
    ))
}
pub async fn set_custom_domain(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let val = body["custom_domain"].as_str().unwrap_or("");
    sqlx::query("INSERT INTO settings (id, tenant_id, key, value) VALUES ($1,$2,'custom_domain',$3) ON CONFLICT (tenant_id, key) DO UPDATE SET value=$3")
        .bind(Uuid::new_v4()).bind(tenant_id).bind(val).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Saved"})))
}

pub async fn get_site_meta(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let row = sqlx::query_as::<_, (Value,)>(
        "SELECT value FROM tenant_settings WHERE tenant_id=$1 AND key='site_meta'",
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(row.map(|r| r.0).unwrap_or(json!({}))))
}

pub async fn set_site_meta(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    crate::features::enforce_feature_limit(&state, tenant_id, "max_cards", "Cards").await?;
    let allowed_keys = [
        "og_title",
        "og_description",
        "og_image",
        "favicon_url",
        "twitter_handle",
        "google_analytics_id",
        "facebook_pixel_id",
        "theme_color",
    ];
    let mut cleaned = serde_json::Map::new();
    if let Some(obj) = body.as_object() {
        for k in allowed_keys {
            if let Some(v) = obj.get(k) {
                cleaned.insert(k.to_string(), v.clone());
            }
        }
    }
    let val = serde_json::Value::Object(cleaned);
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1,$2,'site_meta',$3) ON CONFLICT (tenant_id, key) DO UPDATE SET value=$3"
    ).bind(Uuid::new_v4()).bind(tenant_id).bind(&val).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Saved"})))
}

pub async fn render_card(
    axum::extract::Path(slug): axum::extract::Path<String>,
    axum::extract::Host(host): axum::extract::Host,
    OriginalUri(uri): OriginalUri,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    use sqlx::Row;
    // Extract URL prefix (k/b/c/m/f/h) from the request path
    let path = uri.path().to_string();
    let prefix = path.split('/').nth(1).unwrap_or("k");
    let prefix = if prefix.is_empty() || prefix == slug {
        "k"
    } else {
        prefix
    };
    let row = sqlx::query(
        "SELECT k.id, k.tenant_id, k.title, k.slug, k.bio, k.bg_color, k.accent_color, k.text_color, k.tagline, k.meta_description, k.avatar_url, k.template_type, k.video_provider, k.video_id, k.layout_blocks, k.created_at, k.updated_at, t.affiliate_code, t.settings as tenant_settings, COALESCE(p.features->>'white_label','false') as white_label FROM kinetic_cards k LEFT JOIN tenants t ON t.id = k.tenant_id LEFT JOIN tenant_plans tp ON tp.tenant_id = k.tenant_id AND tp.status = 'active' LEFT JOIN plans p ON p.id = tp.plan_id WHERE k.slug = $1 LIMIT 1"
    ).bind(&slug).fetch_optional(&state.pool).await.unwrap_or(None);

    // ── Load global SEO settings for SSO injection ──
    let seo_rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT key, value FROM site_settings WHERE key LIKE 'seo_%'")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    let mut seo_meta = String::new();
    let mut seo_scripts = String::new();
    for (k, v) in &seo_rows {
        let short = k.strip_prefix("seo_").unwrap_or(k);
        match short {
            "site_name" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!(
                        "<meta property=\"og:site_name\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "description" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta name=\"description\" content=\"{}\">\n", s));
                    seo_meta.push_str(&format!(
                        "<meta property=\"og:description\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "keywords" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta name=\"keywords\" content=\"{}\">\n", s));
                }
            }
            "og_image" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta property=\"og:image\" content=\"{}\">\n", s));
                    seo_meta.push_str(&format!(
                        "<meta property=\"twitter:image\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "twitter_handle" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!("<meta name=\"twitter:site\" content=\"{}\">\n", s));
                    seo_meta.push_str(&format!(
                        "<meta name=\"twitter:creator\" content=\"{}\">\n",
                        s
                    ));
                }
            }
            "site_verification" => {
                if let Some(s) = v.as_str() {
                    seo_meta.push_str(&format!(
                        "<meta name=\"google-site-verification\" content=\"{}\">\n",
                        s
                    ));
                }
            }
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
            "schema_type" => {
                let schema_json = serde_json::to_string(v).unwrap_or_default();
                seo_scripts.push_str(&format!(
                    "<script type=\"application/ld+json\">{}</script>\n",
                    schema_json
                ));
            }
            _ => {}
        }
    }
    // Twitter card type always set
    seo_meta.push_str("<meta property=\"twitter:card\" content=\"summary_large_image\">\n");
    // Canonical URL — resolves from Host header (tenant.kntcrd.com → canonical, custom domain, or fallback)
    let canonical = html_escape(&resolve_canonical_url(&host, prefix, &slug));
    seo_meta.push_str(&format!(
        "<link rel=\"canonical\" href=\"{}\">\n",
        canonical
    ));

    // Font preconnect for speed
    seo_meta.push_str("<link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">\n<link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\n");

    if row.is_none() {
        return axum::response::Html("<html><body style='background:#0f172a;color:#fff;display:flex;align-items:center;justify-content:center;height:100vh;font-family:sans-serif'><div style='text-align:center'><h1 style='font-size:48px;margin-bottom:8px'>404</h1><p>Card not found</p><a href='https://funnelswift.net/kinetic' style='color:#a855f7'>Create your own →</a></div></body></html>".to_string());
    }

    let r = row.unwrap();
    // ── Load tenant-level site meta (overrides card-level OG tags for their subdomain) ──
    let tenant_id_for_meta: Uuid = r.try_get::<Uuid, _>("tenant_id").unwrap_or_default();
    let tenant_site_meta: Value = sqlx::query_as::<_, (Value,)>(
        "SELECT value FROM tenant_settings WHERE tenant_id=$1 AND key='site_meta'",
    )
    .bind(tenant_id_for_meta)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None)
    .map(|r| r.0)
    .unwrap_or(json!({}));
    let _template_type: String = r.try_get("template_type").unwrap_or_default();
    // Branding badge logic:
    //   white_label=true + tenant.settings.hide_branding_badge → hidden
    //   Otherwise → always show (free plans forced ON, paid plans can toggle off)
    let is_white_label: String = r.try_get("white_label").unwrap_or_else(|_| "false".into());
    let tenant_settings: Option<Value> = r.try_get("tenant_settings").unwrap_or(None);
    let tenant_hides_badge = is_white_label == "true"
        && tenant_settings
            .as_ref()
            .and_then(|s| s.get("hide_branding_badge"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let show_branding = !tenant_hides_badge; // always show unless explicitly hidden
    let affiliate_code: Option<String> = r.try_get("affiliate_code").unwrap_or(None);
    let title: String = html_escape(&r.try_get::<String, _>("title").unwrap_or_default());
    let bio: String = html_escape(&r.try_get::<String, _>("bio").unwrap_or_default());
    let bg: String = r.try_get("bg_color").unwrap_or_default();
    let accent: String = r.try_get("accent_color").unwrap_or_default();
    let text: String = r.try_get("text_color").unwrap_or_default();
    let avatar: Option<String> = r
        .try_get::<Option<String>, _>("avatar_url")
        .unwrap_or(None)
        .map(|a| html_escape(&a));
    let tagline: Option<String> = r
        .try_get::<Option<String>, _>("tagline")
        .unwrap_or(None)
        .map(|t| html_escape(&t));
    let meta_desc: Option<String> = r
        .try_get::<Option<String>, _>("meta_description")
        .unwrap_or(None)
        .map(|d| html_escape(&d));
    let card_id: Uuid = r.try_get("id").unwrap_or_default();
    let _video_provider: Option<String> = r.try_get("video_provider").unwrap_or(None);
    let _video_id: Option<String> = r.try_get("video_id").unwrap_or(None);

    // Dynamic branding badge — uses prefix to determine card type label
    let (branding_cta, _card_label) = cta_for_prefix(prefix);
    let (branding_text, branding_url) = if show_branding {
        let url = if let Some(ref code) = affiliate_code {
            format!("https://funnelswift.net/kinetic?ref={}", code)
        } else {
            "https://funnelswift.net/kinetic".to_string()
        };
        (branding_cta.to_string(), url)
    } else {
        (String::new(), String::new())
    };
    let branding_html = if !branding_text.is_empty() {
        format!(
            "<a href='{}' class='branding-badge'>{}</a>",
            branding_url, branding_text
        )
    } else {
        String::new()
    };

    let av_html = if let Some(ref a) = avatar {
        if a.is_empty() {
            String::new()
        } else {
            format!(
                "<img src='{}' class='av' alt='' onerror=\"this.style.display='none'\">",
                a
            )
        }
    } else {
        String::new()
    };
    let tag_html = if let Some(ref t) = tagline {
        if t.is_empty() {
            String::new()
        } else {
            format!("<p class='tag'>{}</p>", t)
        }
    } else {
        String::new()
    };
    let bio_html = if !bio.is_empty() {
        format!("<p class='bio'>{}</p>", bio)
    } else {
        String::new()
    };
    let cta_html = String::new(); // CTA pulled from layout_blocks by front-end JS

    let bg_gradient = format!(
        "radial-gradient(circle at 50% 25%, {}44 0%, {} 70%)",
        accent, bg
    );

    let social_html = String::new(); // social links rendered by front-end JS from layout_blocks

    // ── Tenant site meta overrides for OG tags ──
    let og_title = html_escape(
        tenant_site_meta
            .get("og_title")
            .and_then(|v| v.as_str())
            .unwrap_or(&title),
    );
    let og_desc = html_escape(
        tenant_site_meta
            .get("og_description")
            .and_then(|v| v.as_str())
            .unwrap_or(meta_desc.as_deref().unwrap_or("")),
    );
    let og_image_html = tenant_site_meta.get("og_image")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("<meta property=\"og:image\" content=\"{}\">\n<meta property=\"twitter:image\" content=\"{}\">\n", s, s))
        .or_else(|| {
            let default_img = "<meta property=\"og:image\" content=\"https://funnelswift.net/assets/og-funnelswift-card.png\">\n<meta property=\"og:image:width\" content=\"1200\">\n<meta property=\"og:image:height\" content=\"630\">\n<meta property=\"twitter:image\" content=\"https://funnelswift.net/assets/og-funnelswift-card.png\">\n".to_string();
            Some(default_img)
        })
        .unwrap_or_default();
    let favicon_html = tenant_site_meta
        .get("favicon_url")
        .and_then(|v| v.as_str())
        .map(|s| format!("<link rel=\"icon\" href=\"{}\">\n", s))
        .unwrap_or_default();
    let ga_html = tenant_site_meta.get("google_analytics_id")
        .and_then(|v| v.as_str())
        .map(|s| format!("<script async src=\"https://www.googletagmanager.com/gtag/js?id={0}\"></script><script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{0}');</script>\n", s))
        .unwrap_or_default();
    let fb_html = tenant_site_meta.get("facebook_pixel_id")
        .and_then(|v| v.as_str())
        .map(|s| format!("<script>!function(f,b,e,v,n,t,s){{if(f.bq)return;n=f.bq=function(){{n.callMethod?n.callMethod.apply(n,arguments):n.queue.push(arguments)}};if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';n.queue=[];t=b.createElement(e);t.async=!0;t.src=v;s=b.getElementsByTagName(e)[0];s.parentNode.insertBefore(t,s)}}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','{0}');fbq('track','PageView');</script><noscript><img height=\"1\" width=\"1\" src=\"https://www.facebook.com/tr?id={0}&ev=PageView&noscript=1\"/></noscript>\n", s))
        .unwrap_or_default();
    let twitter_site_meta = tenant_site_meta.get("twitter_handle")
        .and_then(|v| v.as_str())
        .map(|s| format!("<meta name=\"twitter:site\" content=\"@{}\">\n<meta name=\"twitter:creator\" content=\"@{}\">\n", s, s))
        .unwrap_or_default();
    let theme_color_meta = tenant_site_meta
        .get("theme_color")
        .and_then(|v| v.as_str())
        .map(|s| format!("<meta name=\"theme-color\" content=\"{}\">\n", s))
        .unwrap_or_default();

    let page_title_display = og_title.to_string();

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{page_title_display}</title>
<meta name="description" content="{og_desc}">
<meta property="og:title" content="{page_title_display}">
<meta property="og:description" content="{og_desc}">
<meta property="og:type" content="website">
<meta property="og:url" content="{canonical}">
{og_image_html}
{favicon_html}
{twitter_site_meta}
{theme_color_meta}
{seo_meta}
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
.branding-badge{{position:absolute;bottom:16px;right:16px;font-size:11px;padding:6px 14px;border-radius:100px;background:rgba(255,255,255,.1);backdrop-filter:blur(12px);-webkit-backdrop-filter:blur(12px);border:1px solid rgba(255,255,255,.15);color:{text};text-decoration:none;z-index:10;transition:all .2s;font-weight:600;letter-spacing:.3px}}
.branding-badge:hover{{background:rgba(255,255,255,.18);border-color:{accent};color:{accent};transform:translateY(-1px);box-shadow:0 4px 16px {accent}33}}
@keyframes fadeIn{{from{{opacity:0;transform:translateY(16px)}}to{{opacity:1;transform:translateY(0)}}}}
@keyframes pulse-glow{{0%,100%{{box-shadow:0 0 0 4px {accent},0 0 24px {accent}66}}50%{{box-shadow:0 0 0 5px {accent},0 0 36px {accent}88}}}}
</style>
{seo_scripts}
{ga_html}
{fb_html}
<link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<div class="bg"></div>
<div class="noise"></div>
<div class="glow"></div>
{branding_html}
<div class="card">
{av_html}
<h1>{title}</h1>
{tag_html}
{bio_html}
{cta_html}
{social_html}
</div>
<script>
(function(){{try{{var c="{card_id_tracker}";if(!c||c.length<10)return;var a="https://funnelswift.net";var u=navigator.userAgent||"";if(/bot|crawler|spider/i.test(u))return;function t(e,x){{var b={{event_type:e||"view",user_agent:u.substring(0,500),referrer_url:document.referrer||"",device_type:screen.width<768?"mobile":screen.width<1024?"tablet":"desktop",screen_size:(screen.width||0)+"x"+(screen.height||0)}};var p=new URLSearchParams(location.search);["utm_source","utm_medium","utm_campaign","utm_content","utm_term"].forEach(function(k){{var v=p.get(k);if(v)b[k]=v}});if(x)Object.assign(b,x);var r=new XMLHttpRequest();r.open("POST",a+"/card/"+c+"/track",!0);r.setRequestHeader("Content-Type","application/json");r.send(JSON.stringify(b))}}setTimeout(function(){{t("view")}},100);document.addEventListener("visibilitychange",function(){{document.visibilityState==="hidden"&&t("leave")}});document.querySelectorAll("a[href]").forEach(function(e){{e.addEventListener("click",function(){{t("click",{{click_label:(e.textContent||"").trim().substring(0,100),click_url:e.getAttribute("href")||""}})}})}})}}catch(e){{}})}})();
</script>
</body>
</html>"#,
        page_title_display = page_title_display,
        og_desc = og_desc,
        text = text,
        bg_gradient = bg_gradient,
        accent = accent,
        title = title,
        card_id_tracker = card_id,
        canonical = canonical,
        seo_meta = seo_meta,
        seo_scripts = seo_scripts,
        og_image_html = og_image_html,
        favicon_html = favicon_html,
        twitter_site_meta = twitter_site_meta,
        theme_color_meta = theme_color_meta,
        ga_html = ga_html,
        fb_html = fb_html,
        av_html = av_html,
        tag_html = tag_html,
        bio_html = bio_html,
        cta_html = cta_html,
        social_html = social_html,
        branding_html = branding_html
    ))
}
pub async fn submit_lead(
    axum::extract::Path(slug): axum::extract::Path<String>,
    State(_state): State<AppState>,
    Json(_body): Json<Value>,
) -> Json<Value> {
    Json(json!({"message": "Lead submitted", "slug": slug}))
}
pub async fn track_click(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({}))
}
