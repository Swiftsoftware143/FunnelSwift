// Legacy cross-app helper: CoreSwift health.
// Two handlers were removed from this file, both because the callee refused every call:
//  * `sync_coreswift_tag` — deleted 2026-09-25 (kanban t_ae84b186); see the note where it was.
//  * `provision_coreswift_user` — deleted 2026-09-25 (kanban t_5a9e4eb7); see the note below.
// The CoreSwift LEAD push lives in src/coreswift.rs (the single CoreSwift client for
// FunnelSwift) and is exposed at /api/v1/integrations/coreswift/push + /api/v1/push/coreswift.
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};

// Provision a CoreSwift user — REMOVED 2026-09-25 (kanban t_5a9e4eb7).
//
// This handler POSTed `{"name","email","description"}` to
// `{CORESWIFT_URL}/api/admin/portfolio-sync` with only an `x-internal-key` header. That route
// lives in CoreSwift's `admin_actions::router()`, which is layered with `auth_middleware` +
// `require_platform_admin_middleware` (auth::platform_admin, kanban t_d5cf6cad — before it
// existed 6 of 8 admin routes answered an ordinary tenant user), so it wants a platform-admin
// JWT, not the shared internal key. Measured live 2026-09-25: CoreSwift answered
// `401 {"code":401,"error":true,"message":"Authentication required"}` to the real key, and the
// FunnelSwift route therefore answered `200 {"status":"error","coreswift_status":401}` on every
// call — a route whose only possible reply is a failure.
//
// It is not re-pointable: CoreSwift's internal-key surface (the only producer auth FunnelSwift
// holds) creates no users at all. Measured with this handler's own body: `/api/v1/internal/tag-provision`
// -> `422 missing field \`contact\`` (contact-scoped, and it mints a BRAND-NEW tenant),
// `/api/internal/tenants/lookup` -> `400 slug is required` (lookup only),
// `/api/internal/contacts` and `/api/internal/tags` -> `400 tenant_id required` (contact/tag
// scoped, and they need a CoreSwift tenant id — FunnelSwift tenant ids are not CoreSwift tenant
// ids). The handlers that do create a user are the platform-admin router, self-serve
// `/api/auth/*` register and the billing checkout — none of which an internal producer can
// invoke. FunnelSwift's shipped CoreSwift story is per-tenant BYOK (the Integration Center at
// `/api/v1/integrations/coreswift/*`, backed by src/coreswift.rs) and the lead/contact sync into
// CoreSwift; neither mints a CoreSwift *user* — with BYOK the account already exists, which is
// where the API key came from. Nothing in the fleet ever called this route (grep of src, www,
// www-app, the mobile app and the frontends repo history: 0 callers; the sibling
// `/api/v1/push/{workflowswift,adaswift}/user` legs are stubs that fabricate a "provisioned"
// reply). Same class and same treatment as `/api/v1/push/coreswift/tag` (t_ae84b186),
// ADASwift (t_65084d8b) and MissedCallRespondr (t_8803c75e). The route itself was removed from
// api_router.rs; see the note there.

// Sync a tag to CoreSwift — REMOVED 2026-09-25 (kanban t_ae84b186).
// This handler POSTed `{"event","source_app","tags","tenant_id"}` to CoreSwift's
// `/api/v1/webhooks/cross-app/tag-sync`; `TagSyncRequest` also requires
// `lead{id,name,email,company}`, `added_tags`, `removed_tags` and `triggered_by`, so axum's
// `Json` extractor refused the body before the handler ran and the route could never succeed
// (it always answered `{"status":"error","message":"CoreSwift returned status 422"}`). A
// tag-only request has no correct payload — CoreSwift's tag sync is lead/contact-scoped — and
// the tag sync already runs on the canonical path: every lead tag change
// (`/api/v1/leads/:id/tags` -> lead_handler.rs) and plan upgrade (tag_logic.rs) sends the full
// shape. The route itself was removed from api_router.rs; see the note there.

/// Check CoreSwift health
pub async fn coreswift_health(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Value>> {
    let coreswift_url = state.coreswift_url.clone();

    if coreswift_url.is_empty() {
        return Ok(Json(
            json!({"connected":false,"url":null,"status":"not configured"}),
        ));
    }

    let client = reqwest::Client::new();
    let url = format!("{}/api/health", coreswift_url.trim_end_matches('/'));

    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok(Json(
                    json!({"connected":true,"url":coreswift_url,"status":"healthy"}),
                ))
            } else {
                Ok(Json(
                    json!({"connected":false,"url":coreswift_url,"status":format!("status {}", resp.status().as_u16())}),
                ))
            }
        }
        Err(e) => Ok(Json(
            json!({"connected":false,"url":coreswift_url,"status":format!("unreachable: {}", e)}),
        )),
    }
}
