//! The §6 guard test for account closure and data erasure
//! (`docs` standard: account-closure-erasure-standard-2026-11-24.md §6).
//!
//! The standard's six checks are all proved here or next to the code they guard:
//!
//! | §6 | check | where |
//! |----|-------|-------|
//! | 1 | close route with a valid API key -> 401, never 200 | [`human_only_routes_refuse_api_keys`] (both routes are in `HUMAN_ONLY_ROUTES`, the boundary refuses them, and the key resolver marks the credential) |
//! | 2 | close route with no credential -> 401 | [`erasure_routes_are_private_by_default`] (neither is on the public allowlist) |
//! | 3 | neither route appears in `API_KEY_ROUTES` / `INTERNAL_ROUTES` | [`erasure_routes_are_private_by_default`] |
//! | 4 | no MCP tool names close/erase/delete_account | [`no_mcp_tool_can_close_or_erase`] + the fleet MCP server's own test |
//! | 5 | erase with a session and a wrong password -> refused, rows still there | the handler's password arm is asserted in [`erase_handler_keeps_every_lock`]; the live proof runs it |
//! | 6 | successful erase -> rows gone, billing kept, one audit row | the purge contract in [`erase_handler_keeps_every_lock`]; the live proof runs it |
//!
//! Everything here is SOURCE-level on purpose (this fleet's census style): it runs in `cargo test`
//! with no database and no network, and it fails when a LATER change quietly widens the door — which
//! is the failure mode the standard exists to stop. Checks 5 and 6 also have a live proof against a
//! throwaway workspace, recorded on the kanban card; a unit test cannot be that proof.

#[cfg(test)]
mod tests {
    use crate::auth::route_policy::{
        is_human_only_route, is_internal_route, is_public_route, HUMAN_ONLY_ROUTES,
        INTERNAL_ROUTES, PUBLIC_ROUTES,
    };

    /// The router, the handlers and the boundary, read at COMPILE time so the test cannot pass while
    /// looking at a stale copy.
    const ROUTER: &str = include_str!("../api_router.rs");
    const HANDLERS: &str = include_str!("handlers.rs");
    const GLOBAL_AUTH: &str = include_str!("global_auth.rs");
    const API_KEYS: &str = include_str!("../handlers/api_keys.rs");
    /// The customer console (the served SPA) — its MCP screen prints the REAL tool list.
    const SPA: &str = include_str!("../../www-app/index.html");
    /// The migration that owns the purge.
    const MIGRATION: &str = include_str!("../../migrations/141_erasure.sql");

    const CLOSE: &str = "/api/v1/auth/close-account";
    const ERASE: &str = "/api/v1/auth/erase-data";

    /// The body of one `pub async fn <name>` in `handlers.rs`, from its signature to the next
    /// top-level `pub async fn`.
    fn handler_body(name: &str) -> String {
        let needle = format!("pub async fn {name}(");
        let start = HANDLERS
            .find(&needle)
            .unwrap_or_else(|| panic!("{needle} is not in auth::handlers — was it renamed?"));
        let rest = &HANDLERS[start + needle.len()..];
        let end = rest.find("\npub async fn ").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// §6.2 + §6.3 — both routes exist, and neither is reachable without a credential.
    #[test]
    fn erasure_routes_are_private_by_default() {
        for route in [CLOSE, ERASE] {
            assert!(
                ROUTER.contains(&format!("\"{route}\"")),
                "{route} is not mounted in api_router.rs — the guard would be protecting a path \
                 the app no longer serves"
            );
            assert!(
                !is_public_route(route),
                "{route} is on PUBLIC_ROUTES — an anonymous caller could end or wipe an account"
            );
            assert!(
                !is_internal_route(route),
                "{route} is on INTERNAL_ROUTES — the shared internal key could end or wipe an account"
            );
            for (list_name, list) in [
                ("PUBLIC_ROUTES", PUBLIC_ROUTES),
                ("INTERNAL_ROUTES", INTERNAL_ROUTES),
            ] {
                assert!(!list.contains(&route), "{route} appears in {list_name}");
            }
        }
    }

    /// §6.1 — an API KEY (the fleet MCP server's credential, and any agent holding a customer's key)
    /// may never call either door. Proved from three sides: the routes are listed as human-only, the
    /// boundary consults that list, and the credential resolver marks a key so the boundary can tell
    /// it apart from a person's session.
    #[test]
    fn human_only_routes_refuse_api_keys() {
        for route in [CLOSE, ERASE] {
            assert!(
                HUMAN_ONLY_ROUTES.contains(&route),
                "{route} left HUMAN_ONLY_ROUTES — an API key would reach it again"
            );
            assert!(
                is_human_only_route(route),
                "is_human_only_route() does not recognise {route}"
            );
            // A near-miss must NOT be caught: the match is segment-wise, so a future sibling route
            // with a similar name stays an ordinary private route instead of inheriting a refusal
            // it was never given.
            assert!(!is_human_only_route(&format!("{route}-notes")));
        }
        // The boundary is where the refusal lives; a comment alone would not stop a key.
        assert!(
            GLOBAL_AUTH.contains("is_human_only_route(&path)")
                && GLOBAL_AUTH.contains("via_api_key"),
            "global_auth no longer refuses a via_api_key caller on a human-only route"
        );
        // ...and the key resolver is the ONE place that marks a credential as a key.
        assert!(
            API_KEYS.contains("via_api_key: true"),
            "resolve_api_key no longer marks an API-key credential, so the boundary cannot tell it \
             from a person's session"
        );
        // A JWT can never carry the marker: it is only ever set by the key resolver.
        assert!(
            !HANDLERS.contains("via_api_key: true"),
            "a handler must never mint the API-key marker onto a session credential"
        );
    }

    /// §6.4 — no MCP tool may be able to close or erase. The console's MCP screen prints the real
    /// tool list (`MCP_SPECS`), one per fleet AppSpec; the fleet MCP server carries the matching test
    /// over its own catalogue.
    #[test]
    fn no_mcp_tool_can_close_or_erase() {
        let start = SPA
            .find("var MCP_SPECS=[")
            .expect("the MCP screen's tool list is gone");
        let rest = &SPA[start..];
        let end = rest.find("];").expect("unterminated MCP_SPECS");
        let list = &rest[..end];
        assert!(
            list.matches("[\"funnelswift").count() >= 10,
            "the MCP tool list looks truncated — this test would pass on an empty list"
        );
        for word in ["close", "erase", "delete_account", "delete-account"] {
            assert!(
                !list.to_lowercase().contains(word),
                "an MCP tool name mentions {word:?} — an agent could be asked to end or wipe an \
                 account (fleet standard §3.4)"
            );
        }
    }

    /// §6.5 + §6.6 — the handler keeps every lock, server-side, and delegates the purge to the ONE
    /// SQL function that keeps the billing documents and proves the rest are gone.
    #[test]
    fn erase_handler_keeps_every_lock() {
        let body = handler_body("erase_data");
        // The credential must be a person: a key is refused here as well as at the boundary.
        assert!(body.contains("auth.via_api_key"));
        // The typed word is judged server-side.
        assert!(body.contains("eq_ignore_ascii_case(\"DELETE\")"));
        // The password is re-verified — check 5's failure mode is a handler that trusts the session.
        assert!(body.contains("verify_password"));
        // The owner, not any teammate.
        assert!(body.contains("is_workspace_owner"));
        // A platform operator account, an impersonation session, the system tenant and the portfolio
        // are each refused.
        assert!(body.contains("super_admin") && body.contains("impersonating"));
        assert!(body.contains("system_tenant_id()"));
        assert!(body.contains("portfolio_hit"));
        // ONE transaction around the purge, so a failure leaves nothing half-deleted.
        assert!(body.contains("state.pool.begin()") && body.contains("tx.commit()"));
        assert!(body.contains("funnelswift_erase_tenant"));

        // The purge: billing detached, everything else deleted, the audit row written, and a census
        // that REFUSES to report success while a row still names the workspace.
        assert!(
            MIGRATION.contains("UPDATE invoices SET tenant_id = NULL"),
            "the purge no longer keeps billing documents"
        );
        assert!(
            MIGRATION.contains("INSERT INTO account_erasures"),
            "an erasure must leave exactly one audit row"
        );
        assert!(
            MIGRATION.contains("RAISE EXCEPTION") && MIGRATION.contains("still name workspace"),
            "the purge no longer proves the rows are gone before reporting success"
        );
        // The table list is read from the CATALOGUE, so a table added later is purged too instead of
        // being silently left behind.
        assert!(MIGRATION.contains("information_schema.columns"));
    }

    /// The close door is the one this fleet already shipped; the standard's §6 checks 1-3 apply to
    /// it too, and it must keep its own password + word locks.
    #[test]
    fn close_handler_still_keeps_its_locks() {
        let body = handler_body("close_account");
        assert!(body.contains("eq_ignore_ascii_case(\"CLOSE\")"));
        assert!(body.contains("verify_password"));
        assert!(body.contains("super_admin"));
        assert!(HUMAN_ONLY_ROUTES.contains(&CLOSE));
    }
}
