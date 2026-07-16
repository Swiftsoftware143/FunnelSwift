#![allow(dead_code)]

use axum::Router;
use std::net::SocketAddr;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api_router;
mod auth;
mod db;
mod email;
mod error;
mod tag_logic;
mod features;
mod handlers;
#[path = "middleware/mod.rs"]
mod app_middleware;
mod models;
mod templates;
mod security;
mod state;

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

    let pool = database.pool().clone();
    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "default-secret-change-me-in-production".to_string());
    let internal_sync_key = std::env::var("INTERNAL_SYNC_KEY")
        .unwrap_or_else(|_| "".to_string());
    let workflowswift_url = std::env::var("WORKFLOWSWIFT_URL")
        .unwrap_or_else(|_| "http://localhost:8084".to_string());
    let adaswift_url = std::env::var("ADASWIFT_URL")
        .unwrap_or_else(|_| "http://localhost:8087".to_string());
    let coreswift_url = std::env::var("CORESWIFT_URL")
        .unwrap_or_else(|_| "http://localhost:8084".to_string());
    let app_state = AppState::new(pool, jwt_secret, internal_sync_key, workflowswift_url, adaswift_url, coreswift_url);

    // Build router
    let app = create_router(app_state);

    // Get port from environment or default to 8080
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .expect("PORT must be a valid u16");

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
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

    // All routes from api_router and handler-based modules
    Router::new()
        .merge(api_router::create_router(state))
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}
