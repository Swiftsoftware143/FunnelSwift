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
        // Public thank-you page confirmation: GET /api/v1/checkout/session/:id.
        // The buyer lands back from the payment provider with no JWT; the route is
        // id-scoped and only ever returns a non-sensitive summary of that session.
        || path.starts_with("/api/v1/checkout/session/")
        // Public viewer confirmation of a card's consent / age gate:
        // POST /api/v1/kinetic/cards/:id/gate. The viewer of a public card page is
        // anonymous and has no JWT, and the endpoint only ever issues the HMAC for
        // the card's *current* gate configuration (an unconfigured gate gets 400).
        || (path.starts_with("/api/v1/kinetic/cards/") && path.ends_with("/gate"))
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

    let claims = match validate_jwt(&state, &req) {
        Ok(claims) => claims,
        Err(_) => {
            return reject(StatusCode::UNAUTHORIZED, "Authentication required");
        }
    };

    // ── tenants.status enforcement (kanban t_af890bbf) ───────────────────────────────────────
    // This is the ONLY place every authenticated /api/v1 route passes through, so a workspace
    // retired in the admin Tenants screen stops here — including the sessions minted before the
    // retirement, which matters because the JWT lives 30 days (a stateless token cannot be
    // recalled, so the check has to be per request).
    //
    // Fail closed: a retired workspace (`inactive`) and a workspace row that no longer exists
    // both refuse; an unreadable status (Postgres down) refuses too, with a 503 rather than a
    // misleading 403. `role == "admin"` (the platform operator, i.e. the console that owns the
    // flag) is exempt so a mis-ticked workspace can always be switched back — see
    // `tenant_handler::workspace_access_allowed`.
    //
    // Deliberately NOT enforced here — and unreachable from here, because every card-serving
    // surface is public (`is_public`): public card rendering, tracking and lead capture keep
    // working for a retired workspace. See docs/ADMIN_GUIDE.md -> "Workspace Status".
    if claims.role != crate::handlers::tenant_handler::PLATFORM_ADMIN_ROLE {
        let tenant_id = match uuid::Uuid::parse_str(&claims.tenant_id) {
            Ok(id) => id,
            Err(_) => return reject(StatusCode::UNAUTHORIZED, "Invalid tenant in token"),
        };
        match crate::handlers::tenant_handler::tenant_status_for(&state.pool, tenant_id).await {
            Ok(Some(status)) if status == "active" => {}
            Ok(Some(_)) => return reject(StatusCode::FORBIDDEN, WORKSPACE_INACTIVE),
            Ok(None) => return reject(StatusCode::FORBIDDEN, "Workspace no longer exists"),
            Err(e) => {
                tracing::error!("tenant status lookup failed for tenant {tenant_id}: {e}");
                return reject(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Workspace status unavailable",
                );
            }
        }
    }

    next.run(req).await
}

/// The one user-visible string for a retired workspace, shared by every gate so the SPA, the
/// login page and the impersonation handler can all match on it.
pub const WORKSPACE_INACTIVE: &str =
    "This workspace is inactive. Contact support to restore access.";

fn reject(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(json!({ "error": error, "status": status.as_u16() })),
    )
        .into_response()
}

fn validate_jwt(state: &AppState, req: &Request) -> Result<Claims, ()> {
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
    .map(|data| data.claims)
    .map_err(|_| ())
}
