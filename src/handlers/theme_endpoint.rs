//! Theme Library endpoint — serves pre-built theme presets for Kinetic Cards.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::auth::middleware::AuthUser;

// ──────────────────────────────────────────────
// THEME STRUCTS
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeColors {
    pub primary: String,
    pub secondary: String,
    pub accent: String,
    pub background: String,
    pub text: String,
    pub button_bg: String,
    pub button_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeGradient {
    pub colors: Vec<String>,
    pub angle: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeTypography {
    pub font_family: String,
    pub heading_size: String,
    pub body_size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSampleContent {
    pub title: String,
    pub bio: String,
    pub tagline: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub slug: String,
    pub name: String,
    pub niche: String,
    pub description: String,
    pub recommended_layout: String,
    pub colors: ThemeColors,
    pub gradient: ThemeGradient,
    pub typography: ThemeTypography,
    pub sample_content: ThemeSampleContent,
}

/// Load themes from the embedded JSON file.
pub fn load_themes() -> Vec<Theme> {
    let themes_json = include_str!("../../themes.json");
    serde_json::from_str(themes_json).unwrap_or_default()
}

/// GET /api/v1/kinetic/themes — returns theme presets available per plan.
/// Free/Kinetic Free → 3 themes, Pro → 6 themes, Enterprise → all 10.
/// Admin can override per-tenant via tenant_settings (feature_override key).
pub async fn list_themes(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<Theme>>> {
    let tenant_id = uuid::Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant ID".into()))?;

    let all_themes = load_themes();

    // Check for tenant-level override first
    let override_value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT value FROM tenant_settings WHERE tenant_id = $1 AND key = 'feature_override'"
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    let override_limit: Option<i32> = override_value.and_then(|v| {
        v.get("feature_key")
            .and_then(|k| k.as_str())
            .and_then(|key| {
                if key == "kinetic_themes" {
                    v.get("limit_value").and_then(|lv| lv.as_i64()).map(|lv| lv as i32)
                } else {
                    None
                }
            })
    });

    let max_themes = if let Some(limit) = override_limit {
        if limit == -1 { all_themes.len() } else { limit as usize }
    } else {
        // Fall back to plan-based limit
        let limit_result = crate::features::check_feature_limit(
            &state,
            tenant_id,
            "kinetic_themes",
        )
        .await?;

        if limit_result.allowed && limit_result.limit == -1 {
            all_themes.len()
        } else if limit_result.allowed {
            limit_result.limit as usize
        } else {
            3
        }
    };

    let themes: Vec<Theme> = all_themes.into_iter().take(max_themes).collect();
    Ok(Json(themes))
}

// ── TEMPLATES ──

pub fn load_templates() -> Vec<serde_json::Value> {
    let templates_json = include_str!("../../templates.json");
    serde_json::from_str(templates_json).unwrap_or_default()
}

pub async fn list_templates() -> AppResult<Json<serde_json::Value>> {
    let templates = load_templates();
    let grouped = serde_json::json!({
        "templates": templates,
        "types": {
            "business_card": templates.iter().filter(|t| t["type"] == "business_card").count(),
            "bio_link": templates.iter().filter(|t| t["type"] == "bio_link").count(),
            "mini_page": templates.iter().filter(|t| t["type"] == "mini_page").count(),
            "mini_funnel": templates.iter().filter(|t| t["type"] == "mini_funnel").count(),
            "hero": templates.iter().filter(|t| t["type"] == "hero").count(),
            "thank_you": templates.iter().filter(|t| t["type"] == "thank_you").count(),
        },
        "total": templates.len()
    });
    Ok(Json(grouped))
}

