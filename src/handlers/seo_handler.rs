use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use sqlx::Row;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// GET /api/v1/seo/sitemap.xml — dynamic sitemap
pub async fn sitemap_xml(State(state): State<AppState>) -> Response {
    let base = "https://funnelswift.net";
    let mut urls = Vec::new();

    // Static pages
    urls.push(xml_url(base, "", "weekly", "1.0", None));
    urls.push(xml_url(base, "/kinetic", "weekly", "0.9", None));
    urls.push(xml_url(base, "/download-app", "monthly", "0.8", None));
    urls.push(xml_url(base, "/guide", "monthly", "0.7", None));

    // Public kinetic cards
    if let Ok(rows) = sqlx::query(
        "SELECT slug, updated_at FROM kinetic_cards WHERE is_template = false ORDER BY updated_at DESC LIMIT 500"
    )
    .fetch_all(&state.pool)
    .await
    {
        for row in &rows {
            let slug: String = row.get("slug");
            let updated: Option<chrono::NaiveDateTime> = row.get("updated_at");
            let lastmod = updated.map(|d| d.format("%Y-%m-%d").to_string());
            urls.push(xml_url("https://kntcrd.com", &format!("/k/{}", slug), "daily", "0.8", lastmod));
        }
    }

    // Public funnels
    if let Ok(rows) = sqlx::query(
        "SELECT slug, created_at FROM funnels WHERE COALESCE(slug,'') != '' ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(&state.pool)
    .await
    {
        for row in &rows {
            let slug: String = row.get("slug");
            let created: Option<chrono::NaiveDateTime> = row.get("created_at");
            let lastmod = created.map(|d| d.format("%Y-%m-%d").to_string());
            urls.push(xml_url(base, &format!("/funnel/{}", slug), "weekly", "0.7", lastmod));
        }
    }

    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
{}
</urlset>"#,
        urls.join("\n")
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(axum::body::Body::from(body))
        .unwrap()
}

fn xml_url(base: &str, path: &str, changefreq: &str, priority: &str, lastmod: Option<String>) -> String {
    format!(
        r#"  <url>
    <loc>{}{}</loc>
    <changefreq>{}</changefreq>
    <priority>{}</priority>{}
  </url>"#,
        base,
        path,
        changefreq,
        priority,
        lastmod
            .map(|d| format!("\n    <lastmod>{}</lastmod>", d))
            .unwrap_or_default()
    )
}

/// GET /robots.txt — dynamic with configurable crawl rules
pub async fn robots_txt(State(state): State<AppState>) -> Response {
    let base = "https://funnelswift.net";

    // Load crawl-delay from tenant_settings (global key)
    let crawl_delay = sqlx::query_scalar::<_, String>(
        "SELECT value->>'crawl_delay' FROM site_settings WHERE key = 'seo_robots'"
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "1".to_string());

    let body = format!(
        r#"User-agent: *
Allow: /k/
Allow: /funnel/
Allow: /kinetic
Allow: /download-app
Allow: /guide
Disallow: /api/
Disallow: /admin
Disallow: /app
Crawl-delay: {}

Sitemap: {}/api/v1/seo/sitemap.xml
"#,
        crawl_delay, base
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// GET /api/v1/seo/settings — get all SEO-related settings
pub async fn get_seo_settings(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT key, value FROM site_settings WHERE key LIKE 'seo_%' ORDER BY key"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut settings = serde_json::Map::new();
    for (key, value) in rows {
        let short_key = key.strip_prefix("seo_").unwrap_or(&key).to_string();
        settings.insert(short_key, value);
    }
    Ok(Json(Value::Object(settings)))
}

/// PUT /api/v1/seo/settings — update SEO settings
pub async fn update_seo_settings(
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    if let Value::Object(obj) = &payload {
        for (k, v) in obj {
            let key = format!("seo_{}", k);
            sqlx::query(
                "INSERT INTO site_settings (id, key, value) VALUES ($1, $2, $3) ON CONFLICT (key) DO UPDATE SET value = $3"
            )
            .bind(uuid::Uuid::new_v4())
            .bind(&key)
            .bind(v)
            .execute(&state.pool)
            .await?;
        }
    }
    Ok(Json(json!({"message": "SEO settings updated"})))
}

/// GET /api/v1/seo/inject — return meta/script tags for SSR injection
pub async fn seo_inject_tags(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT key, value FROM site_settings WHERE key LIKE 'seo_%' ORDER BY key"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut meta_tags = Vec::new();
    let mut script_tags = Vec::new();

    for (key, value) in rows {
        let short = key.strip_prefix("seo_").unwrap_or(&key);
        match short {
            "site_name" => {
                if let Some(v) = value.as_str() {
                    meta_tags.push(format!("<meta property=\"og:site_name\" content=\"{}\">", v));
                }
            }
            "description" => {
                if let Some(v) = value.as_str() {
                    meta_tags.push(format!("<meta name=\"description\" content=\"{}\">", v));
                    meta_tags.push(format!("<meta property=\"og:description\" content=\"{}\">", v));
                }
            }
            "keywords" => {
                if let Some(v) = value.as_str() {
                    meta_tags.push(format!("<meta name=\"keywords\" content=\"{}\">", v));
                }
            }
            "og_image" => {
                if let Some(v) = value.as_str() {
                    meta_tags.push(format!("<meta property=\"og:image\" content=\"{}\">", v));
                    meta_tags.push(format!("<meta property=\"twitter:image\" content=\"{}\">", v));
                }
            }
            "twitter_handle" => {
                if let Some(v) = value.as_str() {
                    meta_tags.push(format!("<meta name=\"twitter:site\" content=\"{}\">", v));
                    meta_tags.push(format!("<meta name=\"twitter:creator\" content=\"{}\">", v));
                }
            }
            "google_analytics" => {
                if let Some(v) = value.as_str() {
                    script_tags.push(format!(
                        "<script async src=\"https://www.googletagmanager.com/gtag/js?id={}\"></script><script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{}');</script>",
                        v, v
                    ));
                }
            }
            "facebook_pixel" => {
                if let Some(v) = value.as_str() {
                    script_tags.push(format!(
                        "<script>!function(f,b,e,v,n,t,s){{if(f.fbq)return;n=f.fbq=function(){{n.callMethod?n.callMethod.apply(n,arguments):n.queue.push(arguments)}};if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';n.queue=[];t=b.createElement(e);t.async=!0;t.src=v;s=b.getElementsByTagName(e)[0];s.parentNode.insertBefore(t,s)}}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','{}');fbq('track','PageView');</script><noscript><img height=\"1\" width=\"1\" src=\"https://www.facebook.com/tr?id={}&ev=PageView&noscript=1\"/></noscript>",
                        v, v
                    ));
                }
            }
            "site_verification" => {
                if let Some(v) = value.as_str() {
                    meta_tags.push(format!("<meta name=\"google-site-verification\" content=\"{}\">", v));
                }
            }
            "schema_type" => {
                // Stored as JSON: {"type":"Organization","name":"...",...}
                let schema_json = serde_json::to_string(&value).unwrap_or_default();
                script_tags.push(format!("<script type=\"application/ld+json\">{}</script>", schema_json));
            }
            _ => {}
        }
    }

    Ok(Json(json!({
        "meta_tags": meta_tags.join("\n"),
        "script_tags": script_tags.join("\n")
    })))
}
