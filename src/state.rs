use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub db: PgPool,
    pub jwt_secret: String,
    pub internal_sync_key: String,
    pub workflowswift_url: String,
    pub adaswift_url: String,
}

impl AppState {
    pub fn new(pool: PgPool, jwt_secret: String, internal_sync_key: String, workflowswift_url: String, adaswift_url: String) -> Self {
        Self { db: pool.clone(), pool, jwt_secret, internal_sync_key, workflowswift_url, adaswift_url }
    }
}
