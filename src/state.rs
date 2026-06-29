use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub db: PgPool,
    pub jwt_secret: String,
    pub internal_sync_key: String,
}

impl AppState {
    pub fn new(pool: PgPool, jwt_secret: String, internal_sync_key: String) -> Self {
        Self { db: pool.clone(), pool, jwt_secret, internal_sync_key }
    }
}
