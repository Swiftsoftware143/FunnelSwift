use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;
use std::fs;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// GET /api/v1/admin/sites — list all site settings (admin only)
pub async fn list_site_settings(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    if auth.role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let rows = sqlx::query_as::<_, (String, Value)>(
        "SELECT key, value FROM system_settings WHERE key LIKE '%_site_settings' ORDER BY key",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut result = json!({});
    for (key, value) in &rows {
        let slug = key.strip_suffix("_site_settings").unwrap_or(key);
        result[slug] = value.clone();
    }
    Ok(Json(result))
}

/// GET /api/v1/admin/sites/:slug — get a single site's settings (admin only)
pub async fn get_site_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<Json<Value>> {
    if auth.role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let key = format!("{}_site_settings", slug);
    let row: Option<(String, Value)> = sqlx::query_as(
        "SELECT key, value FROM system_settings WHERE key = $1",
    )
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;

    match row {
        Some((_, value)) => Ok(Json(value)),
        None => Err(AppError::NotFound("Site settings not found".into())),
    }
}

/// PUT /api/v1/admin/sites/:slug — update site settings & inject into HTML (admin only)
pub async fn update_site_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    if auth.role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    let key = format!("{}_site_settings", slug);
    sqlx::query(
        r#"INSERT INTO system_settings (id, key, value, created_at, updated_at)
           VALUES ($1, $2, $3, NOW(), NOW())
           ON CONFLICT (key) DO UPDATE SET value = $3, updated_at = NOW()"#,
    )
    .bind(Uuid::new_v4())
    .bind(&key)
    .bind(&body)
    .execute(&state.pool)
    .await?;

    // Inject SEO/tracking settings into the existing HTML file
    inject_into_html(&slug, &body);

    Ok(Json(json!({"message": "Site settings updated", "slug": slug})))
}

fn inject_into_html(slug: &str, settings: &Value) {
    let path = match slug {
        "funnelswift" => "/var/www/funnelswift/index.html",
        "incentiveswift" => "/var/www/incentiveswift-root/index.html",
        "missedcallrespondr" => "/var/www/missedcallrespondr-root/index.html",
        "adaswift" => "/opt/swift/www/adaswift-home/index.html",
        _ => return,
    };

    let html = match fs::read_to_string(path) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("Could not read {}: {}", path, e);
            return;
        }
    };

    let seo = &settings["seo"];
    let tracking = &settings["tracking"];

    let new_title = seo["title"].as_str().unwrap_or("");
    let new_desc = seo["description"].as_str().unwrap_or("");
    let new_keywords = seo["keywords"].as_str().unwrap_or("");
    let new_og_title = seo["og_title"].as_str().unwrap_or("");
    let new_og_desc = seo["og_description"].as_str().unwrap_or("");
    let new_og_image = seo["og_image"].as_str().unwrap_or("");
    let new_canonical = seo["canonical"].as_str().unwrap_or("");
    let new_favicon = seo["favicon_url"].as_str().unwrap_or("");
    let schema_json = seo["schema"].to_string();
    let ga_id = tracking["ga_id"].as_str().unwrap_or("");
    let gtm_id = tracking["gtm_id"].as_str().unwrap_or("");
    let head_extra = tracking["head_scripts"].as_str().unwrap_or("");
    let body_extra = tracking["body_end_scripts"].as_str().unwrap_or("");

    let mut result = html;

    // 1. Replace <title>
    if !new_title.is_empty() {
        result = replace_tag_content(&result, "title", new_title);
    }

    // 2. Replace/ensure meta description
    if !new_desc.is_empty() {
        result = upsert_meta(&result, "description", new_desc);
    }

    // 3. Replace/ensure meta keywords
    if !new_keywords.is_empty() {
        result = upsert_meta(&result, "keywords", new_keywords);
    }

    // 4. OG tags
    if !new_og_title.is_empty() {
        result = upsert_meta_prop(&result, "og:title", new_og_title);
    }
    if !new_og_desc.is_empty() {
        result = upsert_meta_prop(&result, "og:description", new_og_desc);
    }
    if !new_og_image.is_empty() {
        result = upsert_meta_prop(&result, "og:image", new_og_image);
    }

    // 5. Canonical
    if !new_canonical.is_empty() {
        let tag = format!("<link rel=\"canonical\" href=\"{}\">", html_escape(new_canonical));
        result = upsert_link_rel(&result, "canonical", &tag);
    }

    // 6. Favicon
    if !new_favicon.is_empty() {
        let tag = format!("<link rel=\"icon\" href=\"{}\">", html_escape(new_favicon));
        result = upsert_link_rel(&result, "icon", &tag);
    }

    // 7. Schema
    if schema_json != "{}" && schema_json != "null" {
        result = upsert_schema(&result, &schema_json);
    }

    // 8. GA gtag
    if !ga_id.is_empty() {
        let ga_tag = format!(
            r#"<script async src="https://www.googletagmanager.com/gtag/js?id={}"></script>
<script>window.dataLayer=window.dataLayer||[];function gtag(){{dataLayer.push(arguments);}}gtag('js',new Date());gtag('config','{}');</script>"#,
            ga_id, ga_id
        );
        result = inject_before_head_end(&result, &ga_tag);
    }

    // 9. GTM head
    if !gtm_id.is_empty() {
        let gtm_tag = format!(
            r#"<script>(function(w,d,s,l,i){{w[l]=w[l]||[];w[l].push({{'gtm.start':new Date().getTime(),event:'gtm.js'}});var f=d.getElementsByTagName(s)[0],j=d.createElement(s),dl=l!='dataLayer'?'&l='+l:'';j.async=true;j.src='https://www.googletagmanager.com/gtm.js?id='+i+dl;f.parentNode.insertBefore(j,f);}})(window,document,'script','dataLayer','{}');</script>"#,
            gtm_id
        );
        result = inject_before_head_end(&result, &gtm_tag);

        let gtm_body = format!(
            r#"<noscript><iframe src="https://www.googletagmanager.com/ns.html?id={}" height="0" width="0" style="display:none;visibility:hidden"></iframe></noscript>"#,
            gtm_id
        );
        result = inject_after_body_start(&result, &gtm_body);
    }

    // 10. Custom head scripts
    if !head_extra.is_empty() {
        result = inject_before_head_end(&result, head_extra);
    }

    // 11. Custom body end scripts
    if !body_extra.is_empty() {
        result = inject_before_body_end(&result, body_extra);
    }

    if let Err(e) = fs::write(path, &result) {
        tracing::warn!("Failed to write {}: {}", path, e);
    } else {
        tracing::info!("Injected settings into {}", path);
    }
}

// ── HTML Manipulation Helpers ──

fn replace_tag_content(html: &str, tag: &str, new_content: &str) -> String {
    let escaped = html_escape(new_content);
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    if let Some(start) = html.find(&open) {
        let content_start = start + open.len();
        if let Some(end) = html[content_start..].find(&close) {
            let before = &html[..content_start];
            let after = &html[content_start + end..];
            return format!("{}{}{}", before, escaped, after);
        }
    }
    html.to_string()
}

fn upsert_meta(html: &str, name: &str, content: &str) -> String {
    let escaped = html_escape(content);
    // Try to find existing meta name="name"
    let patterns = [
        format!("<meta name=\"{}\" content=\"", name),
        format!("<meta name='{}' content='", name),
        format!("<meta content=\"{}\"", name),
    ];
    for pat in &patterns {
        if let Some(start) = html.find(pat) {
            let close = if pat.contains("'") { "'" } else { "\"" };
            let after_start = start + pat.len();
            if let Some(end) = html[after_start..].find(close) {
                let before = &html[..after_start];
                let after = &html[after_start + end + 1..];
                return format!("{}{}{}", before, escaped, after);
            }
        }
    }
    // Not found, inject after <title>
    let tag = format!("<meta name=\"{}\" content=\"{}\">", name, escaped);
    if let Some(pos) = html.find("</title>") {
        let insert_pos = pos + "</title>".len();
        return format!("{}\n{} {}", &html[..insert_pos], tag, &html[insert_pos..]);
    }
    html.to_string()
}

fn upsert_meta_prop(html: &str, property: &str, content: &str) -> String {
    let escaped = html_escape(content);
    let patterns = [
        format!("<meta property=\"{}\" content=\"", property),
        format!("<meta name=\"{}\" content=\"", property),
    ];
    for pat in &patterns {
        if let Some(start) = html.find(pat) {
            let after_start = start + pat.len();
            if let Some(end) = html[after_start..].find("\"") {
                let before = &html[..after_start];
                let after = &html[after_start + end + 1..];
                return format!("{}{}{}", before, escaped, after);
            }
        }
    }
    // Insert after <title>
    let tag = format!("<meta property=\"{}\" content=\"{}\">", property, escaped);
    if let Some(pos) = html.find("</title>") {
        let insert_pos = pos + "</title>".len();
        return format!("{}\n{} {}", &html[..insert_pos], tag, &html[insert_pos..]);
    }
    html.to_string()
}

fn upsert_link_rel(html: &str, rel: &str, full_tag: &str) -> String {
    let pattern = format!("rel=\"{}\"", rel);
    if let Some(start) = html.find(&pattern) {
        // Find the opening <link
        let line_start = html[..start].rfind('<').unwrap_or(start);
        let line_end = html[start..].find('>').map(|i| start + i + 1).unwrap_or(html.len());
        return format!("{}{}{}", &html[..line_start], full_tag, &html[line_end..]);
    }
    // Insert after <title>
    if let Some(pos) = html.find("</title>") {
        let insert_pos = pos + "</title>".len();
        return format!("{}\n{} {}", &html[..insert_pos], full_tag, &html[insert_pos..]);
    }
    html.to_string()
}

fn upsert_schema(html: &str, schema_json: &str) -> String {
    let start_marker = r#"<script type="application/ld+json">"#;
    let end_marker = r#"</script>"#;
    if let Some(start) = html.find(start_marker) {
        let content_start = start + start_marker.len();
        if let Some(end) = html[content_start..].find(end_marker) {
            let before = &html[..content_start];
            let after = &html[content_start + end..];
            return format!("{}{}{}", before, schema_json, after);
        }
    }
    // Not found, inject before </head>
    let tag = format!("<script type=\"application/ld+json\">{}</script>", schema_json);
    inject_before_head_end(html, &tag)
}

fn inject_before_head_end(html: &str, snippet: &str) -> String {
    if let Some(pos) = html.find("</head>") {
        return format!("{}\n{}{}", &html[..pos], snippet, &html[pos..]);
    }
    html.to_string()
}

fn inject_after_body_start(html: &str, snippet: &str) -> String {
    if let Some(pos) = html.find("<body") {
        // Find the > of the <body tag
        let after = &html[pos..];
        if let Some(close) = after.find('>') {
            let insert_pos = pos + close + 1;
            return format!("{}\n{}\n{}", &html[..insert_pos], snippet, &html[insert_pos..]);
        }
    }
    html.to_string()
}

fn inject_before_body_end(html: &str, snippet: &str) -> String {
    if let Some(pos) = html.rfind("</body>") {
        return format!("{}\n{}\n{}", &html[..pos], snippet, &html[pos..]);
    }
    html.to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
