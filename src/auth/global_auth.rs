use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde_json::json;

use crate::auth::models::Claims;
use crate::state::AppState;

const JWT_ISSUER: &str = "funnelswift";
const JWT_AUDIENCE: &str = "funnelswift-api";

/// Exact public API paths that require no JWT.
const PUBLIC_EXACT: &[&str] = &[
    "/api/health",
    "/api/v1/health",
    "/api/v1/auth/register",
    "/api/v1/auth/signup",
    "/api/v1/auth/login",
    "/api/v1/auth/forgot-password",
    "/api/v1/auth/reset-password",
    // Cross-app webhooks + tracking
    "/api/v1/webhooks/conversion",
    "/api/v1/track/lead",
    "/api/v1/track-click",
    // Public lead capture
    "/api/v1/web-to-lead",
    // Public SEO
    "/api/v1/seo/sitemap.xml",
    "/api/v1/seo/inject",
    // Signature-verified payment webhooks + public checkout
    "/api/v1/webhooks/stripe",
    "/api/v1/webhooks/paypal",
    "/api/v1/checkout/create",
];

fn is_public(path: &str) -> bool {
    if PUBLIC_EXACT.contains(&path) {
        return true;
    }
    // Public embed for web-to-lead forms: GET /api/v1/web-to-lead/configs/:id/embed
    path.starts_with("/api/v1/web-to-lead/configs/") && path.ends_with("/embed")
}

fn is_internal(path: &str) -> bool {
    path.starts_with("/api/v1/internal/")
}

/// Global fail-closed auth: every `/api/v1/*` route not whitelisted requires a valid JWT.
pub async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();

    // Only guard API routes; SSR pages / static assets are public.
    if !path.starts_with("/api/") || is_public(&path) || is_internal(&path) {
        return next.run(req).await;
    }

    match validate_jwt(&state, &req) {
        Ok(_) => next.run(req).await,
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Authentication required"})),
        )
            .into_response(),
    }
}

fn validate_jwt(state: &AppState, req: &Request) -> Result<(), ()> {
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(())?;
    let token = auth.strip_prefix("Bearer ").ok_or(())?;

    let mut validation = Validation::default();
    validation.set_issuer(&[JWT_ISSUER]);
    validation.set_audience(&[JWT_AUDIENCE]);
    validation.validate_exp = true;
    validation.required_spec_claims.clear();
    validation.required_spec_claims.insert("exp".to_string());

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &validation,
    )
    .map_err(|_| ())?;
    Ok(())
}
