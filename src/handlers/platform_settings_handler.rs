//! `GET|PUT /api/v1/admin/platform-settings` — the operator's installation-wide controls.
//!
//! The screen this serves (`renderSettings` in `www-admin/index.html`) was an empty state that named
//! its own missing backend. Both verbs are behind the admin surface gate (`/api/v1/admin/*`), and the
//! handler repeats the role test so the two can never drift: a surface gate protects the class, the
//! handler protects itself.
//!
//! The GET answers the saved values **and** the plans that can legally be a default seat, because a
//! picker must be filled from the server: a hardcoded list in the page goes stale the moment the
//! operator adds or retires a plan.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::platform_settings::{self, PlatformSettings};
use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

fn require_admin(auth: &AuthUser) -> AppResult<()> {
    if auth.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin access required".into()))
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettings {
    pub registration_open: bool,
    #[serde(default)]
    pub default_plan_slug: Option<String>,
    /// Where to mail the operator when a customer files a plan request. ABSENT means "leave the
    /// stored address alone" (a client that predates the field must not wipe it), an EMPTY string
    /// means "stop emailing me", and a value replaces it.
    #[serde(default)]
    pub operator_alert_email: Option<String>,
}

/// `GET /api/v1/admin/platform-settings`
pub async fn get_settings(auth: AuthUser, State(state): State<AppState>) -> AppResult<Json<Value>> {
    require_admin(&auth)?;
    let settings = platform_settings::load(&state.pool).await;
    let plans = platform_settings::seatable_plans(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("platform settings: list plans: {e}")))?;

    // A saved slug that no longer names a seatable plan is REPORTED, not hidden: the signup door
    // ignores an unknown slug (it falls back to the page's own plan), so silence here would leave the
    // operator believing a default was in force when nothing was.
    let default_plan: Option<Value> = match settings.default_plan_slug.as_deref() {
        Some(slug) => plans
            .iter()
            .find(|(s, _, _)| s == slug)
            .map(|(s, n, side)| json!({ "slug": s, "name": n, "side": side, "seat_effective": true }))
            .or_else(|| {
                Some(json!({
                    "slug": slug,
                    "seat_effective": false,
                    "warning": "This plan is no longer an active free plan, so new signups fall \
                                back to the signup page's own plan. Pick another one, or clear it."
                }))
            }),
        None => None,
    };

    Ok(Json(json!({
        "settings": {
            "registration_open": settings.registration_open,
            "default_plan_slug": settings.default_plan_slug,
            "operator_alert_email": settings.operator_alert_email,
        },
        "default_plan": default_plan,
        "seatable_plans": plans
            .into_iter()
            .map(|(slug, name, side)| json!({ "slug": slug, "name": name, "side": side }))
            .collect::<Vec<Value>>(),
        // Stated on the wire, not only in the page, so any client reading this route learns the same
        // scope. Measured reason in platform_settings.rs: one `users` row = one `tenants` row here.
        "not_applicable": [{
            "key": "max_tenants_per_user",
            "reason": "One account owns exactly one workspace in this app, so a per-user workspace \
                       cap has nothing it could refuse."
        }],
    })))
}

/// `PUT /api/v1/admin/platform-settings`
pub async fn update_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateSettings>,
) -> AppResult<Json<Value>> {
    require_admin(&auth)?;

    // An empty string and an absent key both mean "no override" — a browser that clears the picker
    // sends `""`, and treating that as a slug would store a default that can never resolve.
    let slug = body
        .default_plan_slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(ref slug) = slug {
        let seatable = platform_settings::seatable_plans(&state.pool)
            .await
            .map_err(|e| AppError::Internal(format!("platform settings: list plans: {e}")))?;
        if !seatable.iter().any(|(s, _, _)| s == slug) {
            return Err(AppError::Validation(format!(
                "default_plan_slug: {slug} is not a free plan. Only a price-0 plan can be a \
                 default seat — a paid plan would hand a customer a tier nobody paid for."
            )));
        }
    }

    let settings = PlatformSettings {
        registration_open: body.registration_open,
        default_plan_slug: slug,
        // ABSENT keeps the stored address (a client that predates this field must not wipe it);
        // an EMPTY string clears it; anything else must look like an address. Kept looser than any
        // mailer: this refuses only what could never be a destination, so a valid address the
        // provider accepts is never refused here.
        operator_alert_email: match body.operator_alert_email.as_deref().map(str::trim) {
            None => {
                platform_settings::load(&state.pool)
                    .await
                    .operator_alert_email
            }
            Some("") => None,
            Some(v) => {
                if v.contains(char::is_whitespace)
                    || !v.contains('@')
                    || v.starts_with('@')
                    || v.ends_with('@')
                {
                    return Err(AppError::Validation(
                        "operator_alert_email: that is not an email address".into(),
                    ));
                }
                Some(v.to_string())
            }
        },
    };
    platform_settings::save(&state.pool, &settings)
        .await
        .map_err(|e| AppError::Internal(format!("platform settings: save: {e}")))?;

    tracing::info!(
        registration_open = settings.registration_open,
        default_plan_slug = ?settings.default_plan_slug,
        actor = %auth.email,
        "platform settings saved"
    );

    // Answer the SAVED state by reading it back, not by echoing the request: that is what makes the
    // response a statement about the database rather than about the payload.
    let saved = platform_settings::load(&state.pool).await;
    Ok(Json(json!({
        "status": "ok",
        "settings": {
            "registration_open": saved.registration_open,
            "default_plan_slug": saved.default_plan_slug,
            "operator_alert_email": saved.operator_alert_email,
        },
    })))
}
