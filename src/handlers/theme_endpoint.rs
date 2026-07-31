use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::error::AppResult;
use crate::state::AppState;

pub async fn list_themes(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        {
            "id": "cyber_dark",
            "name": "Cyber Dark",
            "primary": "#a855f7",
            "gradient": "linear-gradient(135deg, #0f172a 0%, #1e1b4b 50%, #311042 100%)",
            "bg_color": "#0f172a",
            "accent": "#a855f7",
            "colors": {
                "background": "linear-gradient(135deg, #0f172a 0%, #1e1b4b 50%, #311042 100%)",
                "card_bg": "rgba(255, 255, 255, 0.05)",
                "backdrop_filter": "blur(16px) saturate(180%)",
                "border": "1px solid rgba(168, 85, 247, 0.25)",
                "accent_glow": "0 0 25px rgba(168, 85, 247, 0.4)",
                "text": "#ffffff",
                "text_secondary": "#c4b5fd",
                "button_bg": "rgba(168, 85, 247, 0.25)",
                "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.4), 0 0 0 1px rgba(168, 85, 247, 0.15)"
            }
        },
        {
            "id": "sunset_kinetic",
            "name": "Sunset Kinetic",
            "primary": "#ff4b2b",
            "gradient": "linear-gradient(45deg, #ff416c 0%, #ff4b2b 100%)",
            "bg_color": "#1a0510",
            "accent": "#ff4b2b",
            "colors": {
                "background": "linear-gradient(45deg, #ff416c 0%, #ff4b2b 100%)",
                "card_bg": "rgba(0, 0, 0, 0.2)",
                "backdrop_filter": "blur(12px)",
                "border": "1px solid rgba(255, 255, 255, 0.25)",
                "accent_glow": "0 10px 30px rgba(255, 75, 43, 0.5)",
                "text": "#ffffff",
                "text_secondary": "#ffe4e6",
                "button_bg": "rgba(255, 75, 43, 0.3)",
                "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(255, 75, 43, 0.25), 0 0 0 1px rgba(255, 255, 255, 0.15)"
            }
        },
        {
            "id": "emerald_glass",
            "name": "Emerald Glass",
            "primary": "#10b981",
            "gradient": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
            "bg_color": "#022c22",
            "accent": "#10b981",
            "colors": {
                "background": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
                "card_bg": "rgba(6, 78, 59, 0.3)",
                "backdrop_filter": "blur(20px)",
                "border": "1px solid rgba(52, 211, 153, 0.25)",
                "accent_glow": "0 0 20px rgba(16, 185, 129, 0.35)",
                "text": "#ecfdf5",
                "text_secondary": "#a7f3d0",
                "button_bg": "rgba(16, 185, 129, 0.25)",
                "button_text": "#ecfdf5",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(16, 185, 129, 0.2)"
            }
        },
        {
            "id": "midnight",
            "name": "Midnight",
            "primary": "#6366f1",
            "gradient": "linear-gradient(135deg, #1e1b4b, #312e81, #4c1d95)",
            "bg_color": "#0f172a",
            "accent": "#6366f1",
            "colors": {
                "background": "linear-gradient(135deg, #1e1b4b, #312e81, #4c1d95)",
                "card_bg": "rgba(255, 255, 255, 0.06)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(99, 102, 241, 0.2)",
                "accent_glow": "0 0 20px rgba(99, 102, 241, 0.35)",
                "text": "#ffffff",
                "text_secondary": "#c7d2fe",
                "button_bg": "rgba(99, 102, 241, 0.25)",
                "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.35)"
            }
        },
        {
            "id": "ocean",
            "name": "Ocean",
            "primary": "#0ea5e9",
            "gradient": "linear-gradient(135deg, #0c4a6e, #0369a1, #0284c7)",
            "bg_color": "#0c1929",
            "accent": "#0ea5e9",
            "colors": {
                "background": "linear-gradient(135deg, #0c4a6e, #0369a1, #0284c7)",
                "card_bg": "rgba(14, 165, 233, 0.08)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(14, 165, 233, 0.2)",
                "accent_glow": "0 0 20px rgba(14, 165, 233, 0.3)",
                "text": "#ffffff",
                "text_secondary": "#bae6fd",
                "button_bg": "rgba(14, 165, 233, 0.25)",
                "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.3)"
            }
        },
        {
            "id": "rose",
            "name": "Rose",
            "primary": "#ec4899",
            "gradient": "linear-gradient(135deg, #4c0519, #831843, #be185d)",
            "bg_color": "#1a0510",
            "accent": "#ec4899",
            "colors": {
                "background": "linear-gradient(135deg, #4c0519, #831843, #be185d)",
                "card_bg": "rgba(236, 72, 153, 0.08)",
                "backdrop_filter": "blur(14px)",
                "border": "1px solid rgba(236, 72, 153, 0.2)",
                "accent_glow": "0 0 20px rgba(236, 72, 153, 0.3)",
                "text": "#fce7f3",
                "text_secondary": "#fbcfe8",
                "button_bg": "rgba(236, 72, 153, 0.25)",
                "button_text": "#ffffff",
                "card_shadow": "0 8px 32px rgba(0, 0, 0, 0.3)"
            }
        }
    ])))
}

pub async fn list_templates(State(_state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!([
        {
            "id": "digital_business_card",
            "name": "Digital Business Card",
            "type": "business_card",
            "category": "business_card",
            "niche": "Professional",
            "icon": "BC",
            "description": "Floating avatar with glow border, name/title, 2×2 action grid, glassmorphic bio card, social strip",
            "preview_colors": ["#0f172a", "#a855f7", "#ffffff"],
            "card_type": "business-card",
            "gradient": "radial-gradient(circle at 50% 30%, #1e1b4b 0%, #0f172a 60%, #311042 100%)",
            "theme_key": "cyber_dark",
            "layout_blocks": [
                {"type": "avatar","style": "circle_glow","catchphrase": "Professional Identity"},
                {"type": "action_grid_2x2","items": [{"icon": "📞","label": "Call"},{"icon": "📅","label": "Book"},{"icon": "📧","label": "Email"},{"icon": "💾","label": "Save"}]},
                {"type": "glass_bio","content": "Helping SaaS founders scale through automated acquisition funnels."},
                {"type": "social_strip","platforms": ["linkedin","twitter","instagram"]}
            ]
        },
        {
            "id": "bio_link_creator",
            "name": "Creator Bio Link",
            "type": "bio_link",
            "category": "bio_link",
            "niche": "Creator",
            "icon": "BL",
            "description": "Centered avatar with glow, handle tag, stacked kinetic buttons with hover effects, featured media slot, social row",
            "preview_colors": ["#0c1929", "#0ea5e9", "#ffffff"],
            "card_type": "bio-link",
            "gradient": "linear-gradient(135deg, #0c4a6e 0%, #0369a1 50%, #0c1929 100%)",
            "theme_key": "ocean",
            "layout_blocks": [
                {"type": "avatar","style": "circle_glow","catchphrase": "@yourhandle"},
                {"type": "stacked_links","items": [{"label": "Latest Video","url": "#"},{"label": "Free Course","url": "#"},{"label": "Book a Call","url": "#"},{"label": "Newsletter","url": "#"}]},
                {"type": "featured_card","title": "Featured Product","subtitle": "Check out my new eBook"},
                {"type": "social_row","platforms": ["youtube","twitter","instagram","tiktok"]}
            ]
        },
        {
            "id": "mini_page_showcase",
            "name": "Product Showcase",
            "type": "mini_page",
            "category": "mini_page",
            "niche": "SaaS",
            "icon": "MP",
            "description": "Floating logo bar, bold hero section, 2-column feature grid, trust badge strip, dual CTA buttons",
            "preview_colors": ["#0f172a", "#8b5cf6", "#ffffff"],
            "card_type": "mini-page",
            "gradient": "linear-gradient(135deg, #1e1b4b 0%, #4c1d95 50%, #1e1b4b 100%)",
            "theme_key": "midnight",
            "layout_blocks": [
                {"type": "hero","headline": "Grow Your Business","subtitle": "The all-in-one platform for modern founders","cta_primary": "Get Started","cta_secondary": "Watch Demo"},
                {"type": "feature_grid","items": [{"icon": "🚀","title": "Fast Setup","desc": "Launch in minutes"},{"icon": "📊","title": "Analytics","desc": "Real-time insights"},{"icon": "🔒","title": "Secure","desc": "Enterprise grade"},{"icon": "🤝","title": "Support","desc": "24/7 help"}]},
                {"type": "trust_bar","text":"Trusted by 10k+ businesses · 4.9 ★ Rating"}
            ]
        },
        {
            "id": "conversion_funnel",
            "name": "Conversion Funnel",
            "type": "mini_funnel",
            "category": "mini_funnel",
            "niche": "Marketing",
            "icon": "CF",
            "description": "Urgency badge, massive value headline, focused lead capture form, trust guarantee row, kinetic pulsing CTA",
            "preview_colors": ["#1c1917", "#f59e0b", "#ffffff"],
            "card_type": "mini-funnel",
            "gradient": "linear-gradient(135deg, #451a03 0%, #78350f 50%, #1c1105 100%)",
            "theme_key": "sunset_kinetic",
            "layout_blocks": [
                {"type": "urgency_badge","text": "🔥 Limited Time Access"},
                {"type": "value_headline","headline": "Get Your Free Growth Kit","bullets": ["✓ Custom strategy template","✓ 5 email sequences","✓ Growth calculator"]},
                {"type": "lead_form","fields": ["name","email"],"button": "Send Me The Kit"},
                {"type": "trust_icons","items": ["🔒 Secure","✓ No spam","⭐ 5-star rated"]}
            ]
        },
        {
            "id": "hero_landing",
            "name": "Hero Landing",
            "type": "hero",
            "category": "hero",
            "niche": "Brand",
            "icon": "HP",
            "description": "Full-screen gradient hero with big headline, product image, dual CTA, infinite trust ticker",
            "preview_colors": ["#022c22", "#10b981", "#ecfdf5"],
            "card_type": "hero-page",
            "gradient": "radial-gradient(circle at top left, #064e3b, #022c22, #0f172a)",
            "theme_key": "emerald_glass",
            "layout_blocks": [
                {"type": "hero","headline": "Build Something Great","subtitle": "The modern way to launch and grow","cta_primary": "Start Free","cta_secondary": "Learn More","image_url": ""},
                {"type": "trust_ticker","items": ["🚀 50k+ Users","⭐ 4.9 Rating","🔒 SOC 2","💳 No CC Needed"]}
            ]
        }
    ])))
}
