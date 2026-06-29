use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::time::Duration;

use crate::error::Result;

#[derive(Clone)]
pub struct Database {
    pool: Pool<Postgres>,
}

impl Database {
    pub async fn new() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set");

        let pool = PgPoolOptions::new()
            .max_connections(20)
            .min_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .connect(&database_url)
            .await?;

        tracing::info!("Database connection pool established");

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        tracing::info!("Running database migrations...");
        // Migrations are handled via sqlx migrate or embedded
        // For now, we'll assume migrations are run separately
        Ok(())
    }
}

// FromRef is auto-implemented via axum's blanket impl (T: FromRef<T>) since Rust 1.75+