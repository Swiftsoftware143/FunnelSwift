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
    // Cross-app webhook. Public only because the senders carry no JWT: the handler gates
    // itself on x-internal-key. `/api/v1/track/lead` was deleted with kanban t_f408b7cc
    // (a no-op with no caller in the fleet and no consumer), and `/api/v1/track-click` with
    // kanban t_813d51d4 (a no-op with no caller and no reader of `affiliate_clicks`).
    "/api/v1/webhooks/conversion",
    // Public lead capture
    "/api/v1/web-to-lead",
    // Public SEO
    "/api/v1/seo/sitemap.xml",
    "/api/v1/seo/inject",
    // Public checkout. Public lead capture below.
    "/api/v1/checkout/create",
    // Stripe's payment receiver (David chose Stripe 2026-10-01). Public by necessity: Stripe cannot
    // present a JWT, so the ONLY thing that authenticates a delivery is the Stripe-Signature HMAC —
    // verified in the handler against the tenant's stored signing secret, with the `t=` freshness arm
    // and a per-event idempotency guard. The previous note here ("payment webhook receivers are
    // deliberately NOT public … FunnelSwift has no live checkout") is what left a paid upgrade unable
    // to land: no receiver meant nothing ever called the plan change, so the affiliate was never
    // credited. Revisit only together with the handler in checkout_handler::stripe_webhook.
    "/api/v1/webhooks/stripe",
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

/// The platform-admin role — `users.role = 'admin'`, the role that owns the platform console.
///
/// A tenant-level admin (`company_admin`) is NOT a platform admin: the console at
/// `admin.funnelswift.net` and the admin section of the tenant SPA are both gated on
/// `is_admin` (`role == "admin"`, `auth/middleware.rs`), and every handler that already
/// guarded itself does the same comparison.
pub fn is_platform_admin(role: &str) -> bool {
    role == crate::handlers::tenant_handler::PLATFORM_ADMIN_ROLE
}

const ADMIN_PREFIX: &str = "/api/v1/admin";
const ADMIN_PREFIX_NESTED: &str = "/api/v1/admin/";

/// True when `path` is inside the admin surface. Matches on the path SEGMENT, never on a bare
/// string prefix, so a future `/api/v1/administrators` is not silently swept in.
pub fn is_admin_surface(path: &str) -> bool {
    path == ADMIN_PREFIX || path.starts_with(ADMIN_PREFIX_NESTED)
}

/// The authz decision for the admin surface: `true` means answer 403.
///
/// Before kanban t_9cf378bc the `/api/v1/admin/*` routes were authenticated but never
/// authorised: a token naming a real ACTIVE tenant with `role=user|company_admin` got 200 from
/// GET **and** the write verb of `/api/v1/admin/email-config` — the row that holds the
/// fleet-wide Mailgun private key and the from-address of every platform transactional email.
/// This predicate is applied at the one choke point every admin route passes through
/// (`require_auth`), so the class stays closed for routes added later too.
pub fn admin_surface_denied(path: &str, role: &str) -> bool {
    is_admin_surface(path) && !is_platform_admin(role)
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

    // ── The platform-admin surface (kanban t_9cf378bc) ───────────────────────────────────────
    // Authenticated is not authorised. `/api/v1/admin/*` is PLATFORM-WIDE configuration — the
    // global system-mail credential, the served site, plans, system tags — not tenant data, so a
    // valid token belonging to a tenant must not reach it. Measured live before this check: a
    // token naming a real ACTIVE tenant with `role=user` (and `company_admin`) got 200 from GET
    // and the write verb's 400 (i.e. it reached the handler) on `/api/v1/admin/email-config`,
    // whose row holds the fleet-wide Mailgun private key and the from-address of every platform
    // transactional email — any customer could read or repoint it. Same class as kanban
    // t_d2df1ae1 in ADASwift. Decided BEFORE the tenant-status lookup so a non-admin never pays a
    // DB round trip to be told its role is wrong.
    if admin_surface_denied(&path, &claims.role) {
        return reject(StatusCode::FORBIDDEN, "Platform admin role required");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported defect (kanban t_9cf378bc): a token naming a real ACTIVE tenant with a
    /// non-admin role reached the platform-wide mail config, GET and the write verb.
    #[test]
    fn admin_surface_is_denied_to_tenants_and_allowed_to_the_platform_admin() {
        for role in ["user", "business_user", "company_admin", "member"] {
            assert!(
                admin_surface_denied("/api/v1/admin/email-config", role),
                "{role} must be refused the global mail config"
            );
            assert!(admin_surface_denied("/api/v1/admin/email-templates", role));
            assert!(admin_surface_denied("/api/v1/admin/sites", role));
            assert!(admin_surface_denied("/api/v1/admin/plans", role));
            assert!(admin_surface_denied("/api/v1/admin/site", role));
        }
        // ...while the platform operator still gets through, on every one of those routes.
        for path in [
            "/api/v1/admin/email-config",
            "/api/v1/admin/email-templates/types",
            "/api/v1/admin/sites/funnelswift",
            "/api/v1/admin/plans",
            "/api/v1/admin/site",
        ] {
            assert!(is_admin_surface(path));
            assert!(!admin_surface_denied(path, "admin"));
        }
        // The bare prefix is the surface; a longer word that merely starts the same is not.
        assert!(admin_surface_denied("/api/v1/admin", "user"));
        assert!(!is_admin_surface("/api/v1/administrators"));
        assert!(!is_admin_surface("/api/v1/adminstration"));
        // Tenant-facing routes are untouched by this guard — including the handler that is
        // deliberately SHARED between a tenant route and an admin route
        // (`plan_handler::get_plan`: /api/v1/plans/:id for tenants, /api/v1/admin/plans/:id for
        // the console), and /api/v1/internal/*, which carries its own x-internal-key gate.
        for path in [
            "/api/v1/plans/capture-free",
            "/api/v1/tenants",
            "/api/v1/auth/me",
            "/api/v1/internal/portfolio-sync",
            "/api/health",
        ] {
            assert!(!is_admin_surface(path), "{path} is not the admin surface");
            assert!(!admin_surface_denied(path, "user"));
        }
    }

    #[test]
    fn platform_admin_role_is_exactly_the_one_role_the_console_uses() {
        assert!(is_platform_admin("admin"));
        for role in [
            "user",
            "business_user",
            "company_admin",
            "member",
            "owner",
            "",
        ] {
            assert!(!is_platform_admin(role), "{role} is not a platform admin");
        }
    }
}
