#![recursion_limit = "512"]
#![allow(dead_code)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::needless_return)]

use axum::Router;
use std::net::{IpAddr, SocketAddr};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod affiliate_products;
mod api_router;
#[path = "middleware/mod.rs"]
mod app_middleware;
mod auth;
mod body_deadline;
mod card_blocks;
mod card_types;
mod commission;
mod coreswift;
mod db;
mod email;
mod email_provider;
mod error;
mod feature_registry;
mod features;
mod handlers;
mod models;
mod plan_movement;
mod plan_resolver;
mod probe_harness;
mod security;
mod smtp;
mod state;
mod system_tenant;
mod tag_logic;
mod templates;
mod webhooks;

use crate::db::Database;
use crate::error::Result;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "funnelswift=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Initialize database
    let database = Database::new().await?;

    // Run migrations
    database.migrate().await?;

    // Posture line: a missing master key means BYOK writes fail closed by design. Say so in
    // the boot log instead of discovering it on the first customer write.
    if security::provider_key_crypto::is_configured() {
        tracing::info!("Provider key encryption: enabled (AES-256 at rest, enc:v1 format)");
    } else {
        tracing::error!(
            "Provider key encryption: DISABLED — PROVIDER_KEY_ENC_SECRET missing/short; \
             BYOK writes fail closed (a plaintext credential is never stored)"
        );
    }

    let pool = database.pool().clone();

    // Seal any credential still in the CLEAR in `admin_settings.email` (kanban t_a794cb09).
    // Both write paths seal before they store, and this makes the row converge at boot as well:
    // a database restored from a dump taken before the change — or a writer added later that
    // forgets — otherwise leaves the fleet-wide Mailgun private key readable in a backup.
    // A failure is logged, never fatal: a broken credential row must not stop the app booting.
    match email_provider::seal_legacy_config_secrets(&pool).await {
        Ok(0) => {}
        Ok(n) => tracing::warn!(
            rows = n,
            "admin_settings.email: sealed legacy plaintext credential(s) at rest"
        ),
        Err(e) => tracing::error!(
            "admin_settings.email credential backfill failed (plaintext may remain at rest): {}",
            e
        ),
    }

    // Seal any credential still in the CLEAR in a TENANT's own mail-config rows
    // (`tenant_settings` keys `email_config` / `mailgun_config` / `smtp_config`, kanban t_b040a78e).
    // The tenant settings route seals before it stores; this makes a row that arrives plaintext —
    // from a dump taken before the change, or from a writer added later that forgets — converge at
    // every boot instead of staying readable in a backup. Never fatal.
    match email_provider::seal_legacy_tenant_config_secrets(&pool).await {
        Ok(0) => {}
        Ok(n) => tracing::warn!(
            rows = n,
            "tenant_settings email config: sealed legacy plaintext credential(s) at rest"
        ),
        Err(e) => tracing::error!(
            "tenant_settings email config credential backfill failed (plaintext may remain at \
             rest): {}",
            e
        ),
    }

    // Database-level backstop for the same rows (kanban t_b040a78e item 2): `migrations/084_…`
    // installs the CHECK that refuses a plaintext tenant mail credential, and this re-arms it when
    // it is absent (hand-dropped, or lost in a partial restore) and VALIDATEs it once no unsealed
    // row remains. Runs AFTER the seal above on purpose: the seal is what makes the census zero.
    email_provider::ensure_tenant_config_seal_guard(&pool).await;
    // Boot-time migration verdict (kanban t_c3823fd1): carried into AppState so `GET /api/health`
    // can report (and 503 on) a schema this binary never managed to apply.
    let schema = database.schema().clone();
    let jwt_secret = std::env::var("JWT_SECRET")
        .map_err(|_| std::io::Error::other("JWT_SECRET must be set in environment"))?;
    let internal_sync_key =
        std::env::var("INTERNAL_SYNC_KEY").expect("INTERNAL_SYNC_KEY must be set in environment");
    let workflowswift_url =
        std::env::var("WORKFLOWSWIFT_URL").unwrap_or_else(|_| "http://localhost:8085".to_string());
    let coreswift_url =
        std::env::var("CORESWIFT_URL").unwrap_or_else(|_| "http://localhost:8084".to_string());
    let app_state = AppState::new(
        pool,
        schema,
        jwt_secret,
        internal_sync_key,
        workflowswift_url,
        coreswift_url,
    );

    // Outbound webhook delivery worker (kanban t_431faa99). The delivery row is the queue: this
    // sweeper makes attempt 2 and attempt 3 for rows the request path left `failed`, and recovers a
    // row left `pending`/`sending` by a crash. Without it `max_attempts` and `next_retry_at` — both
    // columns of migrations/0016 — would stay unread configuration, the same defect one level down.
    webhooks::spawn_retry_worker(app_state.pool.clone());

    // Request-body read deadline (kanban t_488a19d4). Printed here so an operator can see the
    // bound that is actually in force: every request that DECLARES a body on a body-carrying
    // method is answered 408 above it; GETs and empty bodies are untouched. BODY_READ_DEADLINE_SECS
    // overrides the default 30 (clamped 5..=300).
    let body_read_deadline = body_deadline::BodyReadDeadline::from_env();
    tracing::info!(
        "Request body-read deadline: {}s on every request that declares a body \
         (POST/PUT/PATCH/DELETE), 408 above that; GET routes read no body and are untouched \
         (BODY_READ_DEADLINE_SECS)",
        body_read_deadline.duration().as_secs()
    );

    // Build router
    let app = create_router(app_state, body_read_deadline);

    // Bind address comes from the deploy env, the same keys the rest of the fleet reads
    // (ADASwift/IncentiveSwift read HOST/PORT; /etc/swift/env/funnelswift.env supplies
    // HOST=127.0.0.1). Before this change the host was a hardcoded INADDR_ANY inside
    // SocketAddr::from, so no env could move the bind off every interface and the
    // operator's old APP_HOST was read by nothing. Defaults unchanged: 0.0.0.0 / 8080.
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .expect("PORT must be a valid u16");

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let ip: IpAddr = host
        .parse()
        .unwrap_or_else(|_| panic!("HOST must be a valid IP address, got {host}"));

    let addr = SocketAddr::from((ip, port));
    tracing::info!("FunnelSwift server starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn create_router(state: AppState, body_read_deadline: body_deadline::BodyReadDeadline) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(std::time::Duration::from_secs(86400));

    let security_mw = axum::middleware::from_fn(app_middleware::security::security_headers);

    Router::new()
        .merge(api_router::create_router(state, body_read_deadline))
        .layer(security_mw)
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}
