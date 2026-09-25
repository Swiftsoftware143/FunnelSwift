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

mod api_router;
#[path = "middleware/mod.rs"]
mod app_middleware;
mod auth;
mod coreswift;
mod db;
mod email;
mod email_provider;
mod error;
mod features;
mod handlers;
mod models;
mod security;
mod smtp;
mod state;
mod tag_logic;
mod templates;

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
        jwt_secret,
        internal_sync_key,
        workflowswift_url,
        coreswift_url,
    );

    // Build router
    let app = create_router(app_state);

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

fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(std::time::Duration::from_secs(86400));

    let security_mw = axum::middleware::from_fn(app_middleware::security::security_headers);

    Router::new()
        .merge(api_router::create_router(state))
        .layer(security_mw)
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}
