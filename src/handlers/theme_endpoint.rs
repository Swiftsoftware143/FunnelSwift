use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::error::AppResult;
use crate::state::AppState;

pub async fn list_themes(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        {"id":"midnight","name":"Midnight","primary":"#6366f1","gradient":"linear-gradient(135deg,#1e1b4b,#312e81,#4c1d95)","bg_color":"#0f172a","accent":"#6366f1"},
        {"id":"ocean","name":"Ocean","primary":"#0ea5e9","gradient":"linear-gradient(135deg,#0c4a6e,#0369a1,#0284c7)","bg_color":"#0c1929","accent":"#0ea5e9"},
        {"id":"sunset","name":"Sunset","primary":"#f59e0b","gradient":"linear-gradient(135deg,#431407,#7c2d12,#b45309)","bg_color":"#1c1917","accent":"#f59e0b"},
        {"id":"emerald","name":"Emerald","primary":"#10b981","gradient":"linear-gradient(135deg,#022c22,#065f46,#059669)","bg_color":"#021a14","accent":"#10b981"},
        {"id":"rose","name":"Rose","primary":"#ec4899","gradient":"linear-gradient(135deg,#4c0519,#831843,#be185d)","bg_color":"#1a0510","accent":"#ec4899"},
        {"id":"amber","name":"Amber","primary":"#d97706","gradient":"linear-gradient(135deg,#451a03,#78350f,#b45309)","bg_color":"#1c1105","accent":"#d97706"}
    ])))
}

pub async fn list_templates(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        {
            "id": "bio_link",
            "name": "Bio Link",
            "category": "bio_link",
            "description": "Centered avatar + bio + stacked link buttons — perfect for Instagram, TikTok, Twitter",
            "card_type": "bio-link",
            "preview": "<div style=\"text-align:center;padding:12px\"><div style=\"width:32px;height:32px;border-radius:50%;background:#6366f1;margin:4px auto\"></div><div style=\"height:6px;width:70%;background:#475569;margin:8px auto;border-radius:3px\"></div><div style=\"height:14px;background:#6366f1;border-radius:4px;margin:6px 0\"></div><div style=\"height:14px;background:#6366f1;border-radius:4px;margin:6px 0\"></div><div style=\"height:14px;background:#6366f1;border-radius:4px;margin:6px 0\"></div></div>",
            "bg_color": "#0f172a",
            "accent_color": "#6366f1",
            "gradient": "linear-gradient(135deg,#1e1b4b,#312e81,#1e1b4b)",
            "blocks": [{"type":"biolink","avatar_url":"","bio":"Your bio here...","buttons":[],"social_links":[]}]
        },
        {
            "id": "business_card",
            "name": "Business Card",
            "category": "business_card",
            "description": "Professional card with photo, title, company, contact info, and social links",
            "card_type": "business-card",
            "preview": "<div style=\"text-align:center;padding:12px\"><div style=\"width:28px;height:28px;border-radius:50%;background:#0ea5e9;margin:4px auto\"></div><div style=\"height:6px;width:60%;background:#7dd3fc;margin:6px auto;border-radius:3px\"></div><div style=\"height:5px;width:40%;background:#38bdf8;margin:4px auto;border-radius:3px\"></div><div style=\"display:flex;gap:4px;margin-top:8px;justify-content:center\"><div style=\"width:16px;height:16px;border-radius:3px;background:#0ea5e9\"></div><div style=\"width:16px;height:16px;border-radius:3px;background:#0ea5e9\"></div><div style=\"width:16px;height:16px;border-radius:3px;background:#0ea5e9\"></div></div></div>",
            "bg_color": "#0c1929",
            "accent_color": "#0ea5e9",
            "gradient": "linear-gradient(135deg,#0c4a6e,#0369a1,#0c4a6e)",
            "blocks": [{"type":"businesscard","name":"Your Name","title":"Your Title","company":"Your Company","phone":"","email":"","website":"","avatar_url":"","social_links":[]}]
        },
        {
            "id": "mini_page",
            "name": "Mini Page",
            "category": "mini_page",
            "description": "Hero section + features grid + lead capture form — your own landing page",
            "card_type": "mini-page",
            "preview": "<div style=\"text-align:center;padding:8px\"><div style=\"height:8px;width:50%;background:rgba(255,255,255,.4);margin:4px auto;border-radius:2px;font-size:6px\">HERO</div><div style=\"display:grid;grid-template-columns:1fr 1fr;gap:3px;margin:4px 0\"><div style=\"height:16px;background:rgba(239,68,68,.15);border-radius:2px\"></div><div style=\"height:16px;background:rgba(239,68,68,.15);border-radius:2px\"></div></div><div style=\"height:8px;background:rgba(99,102,241,.25);border-radius:4px;font-size:6px\">FORM</div></div>",
            "bg_color": "#0f172a",
            "accent_color": "#8b5cf6",
            "gradient": "linear-gradient(135deg,#1e1b4b,#4c1d95,#1e1b4b)",
            "blocks": [{"type":"hero","headline":"Headline","subtitle":"Subtitle","bg_image":"","bg_color":"#0f172a"},{"type":"features","items":[{"icon":"🚀","title":"Feature 1","desc":"Description"},{"icon":"💡","title":"Feature 2","desc":"Description"}]},{"type":"leadform","form_title":"Get Started","button_text":"Submit","placeholder":"your@email.com","fields":["name","email"]}]
        },
        {
            "id": "mini_funnel",
            "name": "Mini Funnel",
            "category": "mini_funnel",
            "description": "Single-product micro-funnel with urgency CTA — perfect for launches and lead magnets",
            "card_type": "mini-funnel",
            "preview": "<div style=\"text-align:center;padding:12px\"><div style=\"width:90%;height:28px;background:#334155;margin:4px auto;border-radius:4px\"></div><div style=\"height:5px;width:50%;background:#fbbf24;margin:6px auto;border-radius:3px\"></div><div style=\"height:14px;background:#ef4444;border-radius:4px;margin:8px 0;font-size:9px;line-height:14px\">GET IT NOW →</div></div>",
            "bg_color": "#0f172a",
            "accent_color": "#f59e0b",
            "gradient": "linear-gradient(135deg,#451a03,#78350f,#451a03)",
            "blocks": [{"type":"minifunnel","product_image":"","product_title":"Amazing Product","product_subtitle":"Short description","cta_text":"Get It Now","cta_url":"https://"}]
        },
        {
            "id": "hero_page",
            "name": "Hero Page",
            "category": "hero",
            "description": "Full-screen gradient hero with product carousel and bold value proposition",
            "card_type": "hero-page",
            "preview": "<div style=\"text-align:center;padding:8px;min-height:60px;display:flex;flex-direction:column;justify-content:center\"><div style=\"height:8px;width:70%;background:rgba(255,255,255,.7);margin:3px auto;border-radius:2px\"></div><div style=\"height:5px;width:50%;background:rgba(255,255,255,.4);margin:3px auto;border-radius:2px\"></div><div style=\"height:12px;width:40%;background:#10b981;border-radius:4px;margin:6px auto;font-size:7px;line-height:12px;color:#fff\">CTA →</div></div>",
            "bg_color": "#0c1929",
            "accent_color": "#10b981",
            "gradient": "linear-gradient(135deg,#022c22,#064e3b,#022c22)",
            "blocks": [{"type":"hero","headline":"Big Bold Headline","subtitle":"Supporting text here","bg_image":"","bg_color":"#0f172a","gradient":"135deg,#4f46e5,#0f172a"},{"type":"leadform","form_title":"Join Now","button_text":"Submit","placeholder":"your@email.com","fields":["name","email"]}]
        }
    ])))
}
