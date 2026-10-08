//! Default-deny routing for the API surface (kanban t_1f29c01f, precedent
//! `IncentiveSwift/src/security/route_policy.rs` @47c909a4).
//!
//! # The rule
//!
//! **A mounted `/api/` route is PRIVATE unless it appears in [`PUBLIC_ROUTES`]**. FunnelSwift
//! already had a global fail-closed gate ([`crate::auth::global_auth::require_auth`], wired in
//! `api_router::create_router`), so — unlike IncentiveSwift, where a handler that forgot its
//! `AuthenticatedUser` extractor answered an anonymous caller — nothing was anonymously reachable
//! by accident. What was *not* committed was the decision: the whitelist lived as an ad-hoc
//! `PUBLIC_EXACT` array plus two `path.starts_with(..)` prefix arms inside the middleware, and one
//! blanket default-allow shape. This module is the committed list, and the middleware now reads
//! only from it.
//!
//! # The census (measured 2026-10-08, from `api_router.rs` source)
//!
//! 187 `.route(..)` mounts / 186 unique paths, classified three ways:
//!
//! ```text
//!   162 mounts  /api/**            161 unique after the duplicate `/api/v1/linkedin/auth`
//!                                 mount (POST + DELETE on one path, legal in axum)
//!     16        public            -> PUBLIC_ROUTES below (15 unique + the health pair)
//!      3        internal          -> INTERNAL_ROUTES below (own shared key at the boundary)
//!    142        private           -> require_auth: app JWT, tenant-status checked
//!                                    (30 of them `/api/v1/admin/**` = the operator surface,
//!                                     additionally role-checked by admin_surface_denied)
//!    25 mounts   served surfaces   -> SSR card/funnel pages, public lead + unlock POSTs,
//!                                    tracking pixel, `/`, `/robots.txt`, `/admin/plans`.
//!                                    Deliberately outside the credential boundary (`is_api_path`);
//!                                    none of them reads a credential and none returns tenant data
//!                                    — they render a public page or accept a public form post
//!                                    keyed by a public slug.
//! ```
//!
//! **No route was found answering an anonymous caller by accident** — the `require_auth` gate was
//! doing its job. The two changes this module makes are structural, and both close a default-allow
//! shape rather than a live leak:
//!
//! 1. `INTERNAL_ROUTES`. `/api/v1/internal/**` used to be a blanket bypass
//!    (`is_internal()` -> `next.run(req)`) that relied on every handler remembering its own
//!    `x-internal-key` check. All three current handlers do check it, but a *new* route under that
//!    prefix would have been anonymous until its author remembered. Now the three routes are named
//!    here and the shared key is required at the boundary, so the fourth one is private by
//!    default.
//! 2. The public arms are **templates matched segment-wise**, not `path.starts_with(..)`. Before,
//!    `path.starts_with("/api/v1/checkout/session/")` made *any* future route under that prefix
//!    public. Now only the mounted `/api/v1/checkout/session/:id` shape is, and a new sibling route
//!    is private until somebody writes it down here.
//!
//! # Credentials accepted
//!
//! An app JWT (`Authorization: Bearer`, HS256 over `JWT_SECRET`, `iss=funnelswift`,
//! `aud=funnelswift-api`) — the only caller credential this app issues; the `api_keys` table and
//! the credential surface were retired (migration 090 / kanban t_5a3c2d9c, t_726416be), so there is
//! no "issued API key" arm here. For [`INTERNAL_ROUTES`] the credential is the app's own shared
//! secret (`INTERNAL_SYNC_KEY`) presented as `X-Internal-Key`. The credential decision never
//! *widens* a caller's reach: the handler still owns authorisation (which tenant, which role), so
//! this can only refuse an anonymous caller.
//!
//! # Adding a route
//!
//! Leave it out of both lists and it is private. Add an entry only when the route must answer a
//! caller that presents no credential — and add the shape to the test module below so the decision
//! and its reason are recorded with the code.

/// Routes that may be reached with NO credential at all.
///
/// Templates use axum's `:param` spelling and match by segment (see [`is_public_route`]), so
/// `/api/v1/checkout/session/:id` accepts `/api/v1/checkout/session/anything` but never
/// `/api/v1/checkout/session/a/b`.
pub const PUBLIC_ROUTES: &[&str] = &[
    // --- liveness ---------------------------------------------------------------
    // Read by the fleet uptime watchdog (`funnelswift:8080:/api/v1/health`) and by
    // `funnelswift-deploy.sh`, which fails the deploy on anything but 200. Both spellings the
    // router mounts are listed.
    "/api/health",
    "/api/v1/health",
    // --- account entry points ---------------------------------------------------
    // The signup flow has to be anonymous by definition: this is where a credential is created.
    "/api/v1/auth/register",
    "/api/v1/auth/signup",
    "/api/v1/auth/login",
    "/api/v1/auth/forgot-password",
    "/api/v1/auth/reset-password",
    // --- public catalogues / served marketing surfaces --------------------------
    "/api/v1/seo/sitemap.xml",
    // The marketing site injects the platform-level `seo_*` rows from `site_settings` (no tenant
    // column) into its HTML; the handler reads only those keys.
    "/api/v1/seo/inject",
    // --- public lead capture / checkout -----------------------------------------
    // The lead-capture form on a served funnel posts here with no session (the visitor is
    // anonymous by construction); the handler scopes the write to the tenant it resolves from the
    // submitted context.
    "/api/v1/web-to-lead",
    // Public embed for a web-to-lead form: `GET /api/v1/web-to-lead/configs/:id/embed`.
    "/api/v1/web-to-lead/configs/:id/embed",
    // A visitor starting a checkout has no session yet; the handler creates the session and scopes
    // it to the tenant named in the request.
    "/api/v1/checkout/create",
    // The buyer lands back from the payment provider with no JWT. The route is session-id-scoped
    // and returns only that session's own non-sensitive summary.
    "/api/v1/checkout/session/:id",
    // Public viewer confirmation of a card's consent / age gate. The viewer is anonymous and has
    // no JWT; the handler issues the HMAC for the card's *current* gate configuration only (an
    // unconfigured gate is refused).
    "/api/v1/kinetic/cards/:id/gate",
    // --- inbound webhooks (their own signature / shared key is the credential) --
    // The fleet's sibling apps (WorkflowSwift, ADASwift, missedcallrespondr) post conversions here
    // and carry no JWT; the handler gates itself on `x-internal-key`.
    "/api/v1/webhooks/conversion",
    // Stripe's receiver. Stripe cannot present a JWT; the ONLY thing that authenticates a delivery
    // is the `Stripe-Signature` HMAC against the tenant's stored signing secret, with the `t=`
    // freshness arm and a per-event idempotency guard, in `checkout_handler::stripe_webhook`.
    "/api/v1/webhooks/stripe",
];

/// Internal (service-to-service) routes: reachable with the app's own shared key and nothing else.
///
/// These are the `/api/v1/internal/**` mounts, previously a blanket bypass. The key is checked at
/// the boundary in `global_auth::require_auth` (constant-time) *in addition to* the handler's own
/// check, so a route added under this prefix without an entry here is refused as an ordinary
/// private route instead of being anonymously reachable.
pub const INTERNAL_ROUTES: &[&str] = &[
    "/api/v1/internal/portfolio-sync",
    "/api/v1/internal/affiliate/upgrade-event",
    "/api/v1/internal/affiliate/apply-rate-bands",
];

/// Is `path` (a concrete request path) one of the committed public templates?
pub fn is_public_route(path: &str) -> bool {
    PUBLIC_ROUTES.iter().any(|t| matches_template(t, path))
}

/// Is `path` (a concrete request path) one of the committed internal templates?
pub fn is_internal_route(path: &str) -> bool {
    INTERNAL_ROUTES.iter().any(|t| matches_template(t, path))
}

/// Does this request path belong to the API surface the credential boundary covers?
///
/// Everything else the router mounts is a SERVED surface: the SSR card/funnel pages, the public
/// lead/unlock form posts, the tracking pixel, `/`, `/robots.txt` and `/admin/plans` (a static
/// Askama shell with no query). Keeping them out of the boundary is the pre-existing, deliberate
/// split (`require_auth` always guarded `/api/` only): a served page carries no tenant data, it
/// renders and then asks the API with the session the browser holds. The census of those 25 mounts
/// is in the module docs.
pub fn is_api_path(path: &str) -> bool {
    path.starts_with("/api/")
}

/// Does one template match one concrete path?
///
/// Segment-wise: the split lengths must agree and every template segment is either a `:param`
/// (any one non-empty segment) or the identical literal. Deliberately stricter than a string
/// prefix — `/api/v1/plansX` is a different route and must not be caught, and a template can never
/// accidentally swallow a longer path.
fn matches_template(template: &str, path: &str) -> bool {
    let t: Vec<&str> = template.split('/').collect();
    let p: Vec<&str> = path.split('/').collect();
    if t.len() != p.len() {
        return false;
    }
    t.iter().zip(p.iter()).all(|(tseg, pseg)| {
        if tseg.strip_prefix(':').is_some() {
            !pseg.is_empty()
        } else {
            tseg == pseg
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        is_api_path, is_internal_route, is_public_route, matches_template, INTERNAL_ROUTES,
        PUBLIC_ROUTES,
    };

    /// The route templates `api_router.rs` mounts, read from the source at compile time. A route
    /// joining or leaving the router changes this set, which is what makes the parity tests below
    /// fail loudly instead of the allowlist silently rotting.
    fn mounted_routes() -> Vec<String> {
        const ROUTER: &str = include_str!("../api_router.rs");
        let bytes = ROUTER.as_bytes();
        let needle = b".route(";
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                let mut j = i + needle.len();
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    let mut k = j + 1;
                    while k < bytes.len() && bytes[k] != b'"' {
                        k += 1;
                    }
                    out.push(ROUTER[j + 1..k].to_string());
                }
                i = j;
            } else {
                i += 1;
            }
        }
        out
    }

    /// Every committed allowlist entry must name a route the router actually mounts. A typo, or an
    /// entry left behind after a route is renamed, is a live hole the moment some other route takes
    /// that path — so it fails here instead.
    #[test]
    fn every_allowlist_entry_names_a_mounted_route() {
        let mounted = mounted_routes();
        assert!(
            mounted.len() > 150,
            "route census found only {} routes — the extractor is broken, not the allowlist",
            mounted.len()
        );
        for entry in PUBLIC_ROUTES.iter().chain(INTERNAL_ROUTES.iter()) {
            assert!(
                mounted.iter().any(|m| m == entry),
                "{entry:?} is not a mounted route (the router's own spelling must match exactly)"
            );
        }
    }

    /// The census the module docs quote: 186 mounts, 161 of them on the API surface. If a route is
    /// added or removed the documented numbers move, so the doc gets re-read.
    #[test]
    fn the_census_shape_is_what_the_docs_say() {
        let mounted = mounted_routes();
        let api = mounted.iter().filter(|p| is_api_path(p)).count();
        assert_eq!(mounted.len(), 187, "mounted route count moved");
        assert_eq!(
            api, 162,
            "/api/ mount count moved — update the census in the module docs"
        );
    }

    /// Neither list has duplicates — a copy-paste mistake here is invisible in production but makes
    /// the parity tests above meaningless.
    #[test]
    fn allowlists_are_unique() {
        for (name, list) in [
            ("PUBLIC_ROUTES", PUBLIC_ROUTES),
            ("INTERNAL_ROUTES", INTERNAL_ROUTES),
        ] {
            let mut sorted: Vec<&str> = list.to_vec();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            assert_eq!(before, sorted.len(), "duplicate entry in {name}");
        }
    }

    /// Every /api route is classified exactly once: public XOR internal XOR private. A route that
    /// is both would be a contradiction the middleware order silently resolves; a route in neither
    /// list is kept so only if its author intended it — this test just proves the two lists never
    /// overlap and that every entry lives on the API surface.
    #[test]
    fn the_two_lists_are_disjoint_and_api_only() {
        for p in PUBLIC_ROUTES {
            assert!(
                is_api_path(p),
                "{p} is on the public list but off the API surface"
            );
            assert!(
                !is_internal_route(p),
                "{p} is on BOTH lists — the internal key would be demanded of a public caller"
            );
        }
        for p in INTERNAL_ROUTES {
            assert!(
                is_api_path(p),
                "{p} is on the internal list but off the API surface"
            );
            assert!(!is_public_route(p), "{p} is on both lists");
        }
    }

    /// Tenant data surfaces and the operator prefix must stay private. These are the routes whose
    /// exposure to an anonymous caller is the whole reason a credential boundary exists, so they
    /// get an explicit negative control rather than relying on the absence of an allowlist entry.
    #[test]
    fn tenant_and_operator_surfaces_are_not_public() {
        for p in [
            "/api/v1/tenants",
            "/api/v1/campaigns",
            "/api/v1/campaigns/0a1b2c3d",
            "/api/v1/leads",
            "/api/v1/bulk/leads",
            "/api/v1/tags",
            "/api/v1/tag-groups",
            "/api/v1/affiliates",
            "/api/v1/affiliate/dashboard",
            "/api/v1/affiliate-payouts",
            "/api/v1/settings",
            "/api/v1/tenant-settings",
            "/api/v1/dashboard",
            "/api/v1/analytics/overview",
            "/api/v1/auth/me",
            "/api/v1/auth/profile",
            "/api/v1/ocr/scans",
            "/api/v1/portfolio-companies",
            "/api/v1/provider-keys",
            "/api/v1/web-to-lead/configs",
            "/api/v1/checkout/sessions",
            "/api/v1/available-providers",
            // operator surface
            "/api/v1/admin",
            "/api/v1/admin/plans",
            "/api/v1/admin/tenants",
            "/api/v1/admin/email-config",
            "/api/v1/admin/site",
            // internal, but reached with the shared key and not anonymously
            "/api/v1/internal/portfolio-sync",
            "/api/v1/internal/affiliate/upgrade-event",
            "/api/v1/internal/affiliate/apply-rate-bands",
            // a NEW route under the old blanket prefix must be private, not a bypass
            "/api/v1/internal/something-new",
        ] {
            assert!(!is_public_route(p), "{p} must be private by default");
        }
    }

    /// The other side: the surfaces the app's own signup flow, the anonymous visitor and the
    /// webhook senders need must stay reachable with no credential. A blanket gate is the failure
    /// mode this list exists to stop — it would take down signup and every public card page.
    #[test]
    fn public_surfaces_stay_public() {
        for p in [
            "/api/health",
            "/api/v1/health",
            "/api/v1/auth/register",
            "/api/v1/auth/signup",
            "/api/v1/auth/login",
            "/api/v1/auth/forgot-password",
            "/api/v1/auth/reset-password",
            "/api/v1/seo/sitemap.xml",
            "/api/v1/seo/inject",
            "/api/v1/web-to-lead",
            "/api/v1/web-to-lead/configs/0a1b2c3d/embed",
            "/api/v1/checkout/create",
            "/api/v1/checkout/session/abc123",
            "/api/v1/kinetic/cards/0a1b2c3d/gate",
            "/api/v1/webhooks/conversion",
            "/api/v1/webhooks/stripe",
        ] {
            assert!(is_public_route(p), "{p} must stay reachable anonymously");
        }
    }

    /// The internal surface is reachable ONLY with the shared key, so it must never be on the
    /// public list, and its templates must be matched exactly.
    #[test]
    fn internal_routes_are_named_and_not_public() {
        for p in [
            "/api/v1/internal/portfolio-sync",
            "/api/v1/internal/affiliate/upgrade-event",
            "/api/v1/internal/affiliate/apply-rate-bands",
        ] {
            assert!(is_internal_route(p), "{p} must be an internal route");
            assert!(!is_public_route(p), "{p} must not be anonymous");
        }
        // ...but a sibling under the prefix that nobody named is an ordinary private route.
        assert!(!is_internal_route("/api/v1/internal/affiliate/nope"));
        assert!(!is_internal_route(
            "/api/v1/internal/affiliate/apply-rate-bands/extra"
        ));
    }

    /// Segment matching, not string prefixing — the exact defect this module fixes: a
    /// `starts_with` arm let ANY sibling under the checkout-session prefix answer anonymously.
    #[test]
    fn matching_is_segment_exact() {
        assert!(matches_template(
            "/api/v1/checkout/session/:id",
            "/api/v1/checkout/session/abc"
        ));
        assert!(!matches_template(
            "/api/v1/checkout/session/:id",
            "/api/v1/checkout/session/abc/refund"
        ));
        assert!(!matches_template(
            "/api/v1/checkout/session/:id",
            "/api/v1/checkout/session"
        ));
        assert!(matches_template(
            "/api/v1/web-to-lead",
            "/api/v1/web-to-lead"
        ));
        assert!(!matches_template(
            "/api/v1/web-to-lead",
            "/api/v1/web-to-lead/configs"
        ));
        assert!(!matches_template(
            "/api/v1/web-to-lead",
            "/api/v1/web-to-leadX"
        ));
        assert!(matches_template(
            "/api/v1/web-to-lead/configs/:id/embed",
            "/api/v1/web-to-lead/configs/7/embed"
        ));
        assert!(!matches_template(
            "/api/v1/web-to-lead/configs/:id/embed",
            "/api/v1/web-to-lead/configs/7/embed/x"
        ));
        assert!(matches_template("/api/v1/plans", "/api/v1/plans"));
        assert!(!matches_template("/api/v1/plans", "/api/v1/plans/extra"));
        assert!(!matches_template("/api/v1/plans", "/api/v1/plays"));
        // a `:param` never matches an empty segment
        assert!(!matches_template("/api/v1/play/:id", "/api/v1/play/"));
    }

    /// The served surfaces are outside the API boundary by construction. This pins the boundary
    /// itself so a change to `is_api_path` cannot silently pull the SSR pages into (or drop the
    /// API out of) the credential gate.
    #[test]
    fn served_surfaces_are_outside_the_api_boundary() {
        for p in [
            "/",
            "/robots.txt",
            "/admin/plans",
            "/k/some-slug",
            "/b/some-slug",
            "/funnel/some-slug",
            "/k/some-slug/lead",
            "/k/some-slug/unlock",
            "/card/0a1b2c3d/track",
        ] {
            assert!(!is_api_path(p), "{p} is a served surface, not an API path");
            assert!(!is_public_route(p));
        }
        assert!(is_api_path("/api/v1/health"));
    }
}
