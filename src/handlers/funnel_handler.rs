use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn list_funnels(_auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, COALESCE(slug,''), created_at FROM funnels ORDER BY created_at DESC"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    Ok(Json(json!(rows)))
}
pub async fn create_funnel(auth: AuthUser, State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    let id = Uuid::new_v4();
    let name = payload["name"].as_str().unwrap_or("New Funnel");
    let tenant_id = Uuid::parse_str(&auth.tenant_id).unwrap_or_default();
    sqlx::query("INSERT INTO funnels (id, tenant_id, name, slug) VALUES ($1, $2, $3, $4)")
        .bind(id).bind(tenant_id).bind(name).bind(name.to_lowercase().replace(' ', "-")).execute(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id.to_string(), "message": "Funnel created"}))))
}
pub async fn get_funnel(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let row: Option<(Uuid, String, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, name, COALESCE(slug,''), created_at FROM funnels WHERE id = $1"
    ).bind(id).fetch_optional(&state.pool).await?;
    let r = row.ok_or_else(|| AppError::NotFound("Funnel not found".into()))?;
    Ok(Json(json!({"id": r.0.to_string(), "name": r.1, "slug": r.2})))
}
pub async fn update_funnel(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>, Json(payload): Json<Value>) -> AppResult<Json<Value>> {
    if let Some(name) = payload["name"].as_str() { sqlx::query("UPDATE funnels SET name=$1 WHERE id=$2").bind(name).bind(id).execute(&state.pool).await?; }
    Ok(Json(json!({"message": "Funnel updated"})))
}
pub async fn delete_funnel(_auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    sqlx::query("DELETE FROM funnels WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(Json(json!({"message": "Funnel deleted"})))
}
pub async fn render_funnel(axum::extract::Path(slug): axum::extract::Path<String>, State(state): State<AppState>) -> axum::response::Html<String> {
    // Load SEO settings for injection
    use sqlx::Row;
    let seo_rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT key, value FROM site_settings WHERE key LIKE 'seo_%'"
    ).fetch_all(&state.pool).await.unwrap_or_default();
    let mut seo_meta = String::from("<meta name=\"description\" content=\"FunnelSwift — interactive funnel for leads and conversion.\">\n<meta property=\"og:type\" content=\"website\">\n<meta property=\"twitter:card\" content=\"summary_large_image\">\n");
    let mut seo_scripts = String::new();
    for (k, v) in &seo_rows {
        let short = k.strip_prefix("seo_").unwrap_or(k);
        match short {
            "google_analytics" => { if let Some(s) = v.as_str() { seo_scripts.push_str(&format!("<script async src=\"https://www.googletagmanager.com/gtag/js?id={}\"></script><script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{}');</script>\n", s, s)); } }
            "facebook_pixel" => { if let Some(s) = v.as_str() { seo_scripts.push_str(&format!("<script>!function(f,b,e,v,n,t,s){{if(f.fbq)return;n=f.fbq=function(){{n.callMethod?n.callMethod.apply(n,arguments):n.queue.push(arguments)}};if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';n.queue=[];t=b.createElement(e);t.async=!0;t.src=v;s=b.getElementsByTagName(e)[0];s.parentNode.insertBefore(t,s)}}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','{}');fbq('track','PageView');</script><noscript><img height=\"1\" width=\"1\" src=\"https://www.facebook.com/tr?id={}&ev=PageView&noscript=1\"/></noscript>\n", s, s)); } }
            _ => {}
        }
    }
    seo_meta.push_str(&format!("<link rel=\"canonical\" href=\"https://funnelswift.net/funnel/{}\">\n", slug));
    seo_meta.push_str("<link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">\n<link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\n");

    // Try to get the funnel row
    let funnel_name = sqlx::query_scalar::<_, String>("SELECT name FROM funnels WHERE slug = $1")
        .bind(&slug).fetch_optional(&state.pool).await.unwrap_or(None)
        .unwrap_or_else(|| slug.clone());

    axum::response::Html(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{funnel_name} — FunnelSwift</title>
{seo_meta}
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:'Plus Jakarta Sans','Inter',system-ui,-apple-system,sans-serif;background:linear-gradient(135deg,#1e1b4b 0%,#312e81 100%);color:#fff;min-height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;padding:24px}}
h1{{font-size:32px;margin-bottom:8px}}
p{{color:#c7d2fe;font-size:16px}}
</style>
{seo_scripts}
</head>
<body>
<h1>{funnel_name}</h1>
<p>This funnel is under construction. Check back soon!</p>
</body>
</html>"#, funnel_name = funnel_name, seo_meta = seo_meta, seo_scripts = seo_scripts))
}
