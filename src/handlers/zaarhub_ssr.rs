/// SSR handlers for ZaarHub city landing pages
use axum::extract::{Path, State};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::state::AppState;

/// Simple HTML escaper
fn h(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Render a full city landing page (SEO-optimized SSR HTML)
pub async fn render_city_page(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let city_row = sqlx::query(
        "SELECT id, city_slug, city_name, state, description, hero_image_url, meta_title, meta_description \
         FROM city_pages WHERE city_slug = $1 AND is_active = true",
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    if city_row.is_none() {
        return axum::response::Html(
            r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0"><title>404 — City Not Found | ZaarHub</title><style>body{font-family:system-ui,sans-serif;background:#f8f9fc;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}h1{font-size:48px;color:#2b3255}p{color:#6b7280}a{color:#f27f2f}</style></head><body><div style="text-align:center"><h1>404</h1><p>This city page doesn't exist yet.</p><a href="/zaarhub">Browse all cities →</a></div></body></html>"#
                .to_string(),
        );
    }

    let r = city_row.unwrap();
    let city_name: String = r.try_get("city_name").unwrap_or_default();
    let hero_img: Option<String> = r.try_get("hero_image_url").unwrap_or(None);
    let meta_title: Option<String> = r.try_get("meta_title").unwrap_or(None);
    let meta_desc: Option<String> = r.try_get("meta_description").unwrap_or(None);
    let city_page_id: Uuid = r.try_get("id").unwrap_or_default();

    let page_title = meta_title.unwrap_or_else(|| format!("Best Businesses in {} | ZaarHub", city_name));
    let page_desc = meta_desc.unwrap_or_else(|| format!("Find top-rated local businesses in {}. Browse reviews, deals, and more.", city_name));

    // Load top featured listings for this city
    let rows = sqlx::query(
        "SELECT bl.* FROM business_listings bl \
         WHERE bl.city_page_id = $1 AND bl.is_featured = true \
         ORDER BY bl.rating DESC NULLS LAST LIMIT 24",
    )
    .bind(city_page_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut listings_html = String::new();
    for l in &rows {
        let name: String = l.try_get("business_name").unwrap_or_default();
        let cat: Option<String> = l.try_get("category").unwrap_or(None);
        let desc: Option<String> = l.try_get("description").unwrap_or(None);
        let addr: Option<String> = l.try_get("address").unwrap_or(None);
        let logo: Option<String> = l.try_get("logo_url").unwrap_or(None);
        let rating: Option<f64> = l.try_get("rating").unwrap_or_default();
        let reviews: i32 = l.try_get("review_count").unwrap_or(0);
        let lid: Uuid = l.try_get("id").unwrap_or_default();
        let r = rating.unwrap_or(0.0);
        let stars = String::from("★".repeat(r as usize)) + &"☆".repeat(5usize.saturating_sub(r as usize));

        let logo_html = match &logo {
            Some(img) if !img.is_empty() => format!(
                "<img src=\"{}\" class=\"logo-img\" alt=\"\" loading=\"lazy\" onerror=\"this.style.display='none';this.nextElementSibling.style.display='flex'\"><div class=\"logo-placeholder\" style=\"display:none\">{}</div>",
                h(img), h(&name[..1.min(name.len())])
            ),
            _ => format!("<div class=\"logo-placeholder\">{}</div>", h(&name[..1.min(name.len())])),
        };

        let cat_html = cat
            .as_ref()
            .map(|c| format!("<span class=\"category-tag\">{}</span>", h(c)))
            .unwrap_or_default();
        let desc_html = desc
            .as_ref()
            .map(|d| format!("<p class=\"desc\">{}</p>", h(d)))
            .unwrap_or_default();
        let addr_html = addr
            .as_ref()
            .map(|a| format!("<span>📍 {}</span>", h(a)))
            .unwrap_or_default();

        listings_html.push_str(&format!(
            r#"<a href="/zaarhub/{slug}/{lid}" class="listing-card">
      {logo_html}
      <div class="info">
        <h3>{name}</h3>
        {cat_html}
        {desc_html}
        <div class="meta">
          <span class="stars">{stars}</span>
          <span>{rating:.1}</span>
          <span>{reviews} reviews</span>
          {addr_html}
        </div>
      </div>
    </a>
"#,
            slug = h(&slug),
            lid = lid,
            logo_html = logo_html,
            name = h(&name),
            cat_html = cat_html,
            desc_html = desc_html,
            stars = stars,
            rating = r,
            reviews = reviews,
            addr_html = addr_html,
        ));
    }

    let hero_section = match &hero_img {
        Some(img) if !img.is_empty() => format!(
            "<div class=\"hero\" style=\"background:linear-gradient(rgba(43,50,85,.85),rgba(43,50,85,.92)),url({}) center/cover\"><h1>Best Businesses in <span>{}</span></h1><p>{}</p></div>",
            h(img), h(&city_name), h(&page_desc)
        ),
        _ => format!(
            "<div class=\"hero\"><h1>Best Businesses in <span>{}</span></h1><p>{}</p></div>",
            h(&city_name), h(&page_desc)
        ),
    };

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>{title}</title>
<meta name="description" content="{desc}">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{desc}">
<meta property="og:type" content="website">
<meta property="twitter:card" content="summary_large_image">
<link rel="canonical" href="https://zaarhub.com/{slug}">
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.5}}
header{{background:#2b3255;color:white;padding:16px 20px;position:sticky;top:0;z-index:100}}
header .inner{{max-width:1200px;margin:0 auto;display:flex;justify-content:space-between;align-items:center}}
header .logo{{font-size:22px;font-weight:800;color:white;text-decoration:none}}header .logo span{{color:#f27f2f}}
.hero{{padding:56px 20px;text-align:center;color:white;background:linear-gradient(135deg,#2b3255,#1a1a3e)}}
.hero h1{{font-size:clamp(26px,5vw,38px);margin-bottom:8px}}.hero h1 span{{color:#f27f2f}}.hero p{{opacity:.85;max-width:600px;margin:0 auto}}
.listing-grid{{max-width:1200px;margin:32px auto;padding:0 20px;display:grid;gap:16px}}
.listing-card{{display:flex;gap:16px;align-items:flex-start;padding:20px;background:white;border-radius:14px;box-shadow:0 1px 3px rgba(0,0,0,.06);text-decoration:none;color:inherit;transition:all .2s;border:2px solid transparent}}
.listing-card:hover{{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}}
.logo-img{{width:64px;height:64px;border-radius:14px;object-fit:cover;flex-shrink:0}}
.logo-placeholder{{width:64px;height:64px;border-radius:14px;background:#f27f2f;color:white;display:flex;align-items:center;justify-content:center;font-size:24px;font-weight:700;flex-shrink:0}}
.info{{flex:1;min-width:0}}
.info h3{{font-size:17px;font-weight:700;margin-bottom:2px}}
.category-tag{{display:inline-block;font-size:11px;font-weight:600;text-transform:uppercase;color:#f27f2f;background:#fff7f0;padding:3px 10px;border-radius:6px;margin-bottom:6px;margin-right:6px}}
.desc{{font-size:13px;color:#6b7280;line-height:1.6;margin-bottom:8px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}
.meta{{display:flex;gap:12px;flex-wrap:wrap;font-size:12px;color:#6b7280;align-items:center}}
.stars{{color:#f59e0b}}
footer{{text-align:center;padding:48px 20px;color:#6b7280;font-size:13px}}footer a{{color:#f27f2f;text-decoration:none}}
.load-more{{text-align:center;margin:24px 0}}
.load-more a{{display:inline-block;padding:14px 32px;background:#2b3255;color:white;border-radius:100px;text-decoration:none;font-weight:600;font-size:14px;transition:all .2s}}
.load-more a:hover{{background:#f27f2f;transform:translateY(-1px)}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">Zaar<span>Hub</span></a><nav><a href="/zaarhub-city.html" style="color:rgba(255,255,255,.8);text-decoration:none;font-size:14px;font-weight:500">🔍 Search</a></nav></div></header>
{hero}
<div class="listing-grid">{listings}</div>
<div class="load-more"><a href="/zaarhub-city.html?city={slug}">View all {city_name} businesses →</a></div>
<footer><p>Powered by <a href="https://funnelswift.net">FunnelSwift</a> · <a href="/zaarhub">All Cities</a></p></footer>
</body>
</html>"#,
        title = h(&page_title),
        desc = h(&page_desc),
        slug = h(&slug),
        city_name = h(&city_name),
        hero = hero_section,
        listings = listings_html,
    ))
}

/// Render the all-cities index page (SSR)
pub async fn render_cities_index(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT cp.city_slug, cp.city_name, cp.description, \
                (SELECT COUNT(*) FROM business_listings WHERE city_page_id = cp.id) AS listing_count \
         FROM city_pages cp WHERE cp.is_active = true ORDER BY cp.city_name",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut cities_html = String::new();
    for r in &rows {
        let slug: String = r.try_get("city_slug").unwrap_or_default();
        let name: String = r.try_get("city_name").unwrap_or_default();
        let desc: Option<String> = r.try_get("description").unwrap_or(None);
        let count: i64 = r.try_get("listing_count").unwrap_or(0);
        cities_html.push_str(&format!(
            r#"<a href="/zaarhub/{slug}" class="city-card">
      <h2>{name} <span class="count">{count}+</span></h2>
      <p>{desc}</p>
      <span class="arrow">Browse →</span>
    </a>
"#,
            slug = h(&slug),
            name = h(&name),
            count = count,
            desc = h(&desc.unwrap_or_default()),
        ));
    }

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>ZaarHub — Florida Local Business Directory</title>
<meta name="description" content="Browse 9 Florida cities with thousands of top-rated local businesses. Find restaurants, services, shops, and more.">
<meta property="og:title" content="ZaarHub — Florida Local Business Directory">
<meta property="og:type" content="website">
<meta property="twitter:card" content="summary_large_image">
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.5}}
header{{background:#2b3255;color:white;padding:16px 20px;text-align:center}}
header .logo{{font-size:22px;font-weight:800}}header .logo span{{color:#f27f2f}}
.hero{{padding:64px 20px 48px;text-align:center;background:linear-gradient(135deg,#2b3255,#1a1a3e);color:white}}
.hero h1{{font-size:clamp(28px,5vw,42px);margin-bottom:8px}}.hero h1 span{{color:#f27f2f}}
.hero p{{opacity:.85;max-width:600px;margin:0 auto;font-size:16px}}
.city-grid{{max-width:1000px;margin:32px auto;padding:0 20px;display:grid;gap:20px;grid-template-columns:repeat(auto-fill,minmax(280px,1fr))}}
.city-card{{display:block;padding:24px;background:white;border-radius:14px;box-shadow:0 1px 3px rgba(0,0,0,.06);text-decoration:none;color:inherit;transition:all .2s;border:2px solid transparent}}
.city-card:hover{{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}}
.city-card h2{{font-size:20px;font-weight:700;margin-bottom:4px}}
.city-card h2 .count{{display:inline-block;background:#fff7f0;color:#f27f2f;font-size:12px;padding:3px 10px;border-radius:10px;margin-left:8px;vertical-align:middle}}
.city-card p{{font-size:14px;color:#6b7280;margin-bottom:12px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}
.city-card .arrow{{font-size:13px;color:#f27f2f;font-weight:600}}
footer{{text-align:center;padding:48px 20px;color:#6b7280;font-size:13px}}footer a{{color:#f27f2f;text-decoration:none}}
@media(max-width:600px){{.city-grid{{grid-template-columns:1fr}}}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><span class="logo">Zaar<span>Hub</span></span></header>
<div class="hero"><h1>Florida <span>Business Directory</span></h1><p>Browse top-rated local businesses across 9 Florida cities with thousands of listings, reviews, and deals.</p></div>
<div class="city-grid">{cities}</div>
<footer><p>Powered by <a href="https://funnelswift.net">FunnelSwift</a> · <a href="/zaarhub-city.html">Search All Cities</a></p></footer>
</body>
</html>"#,
        cities = cities_html,
    ))
}
