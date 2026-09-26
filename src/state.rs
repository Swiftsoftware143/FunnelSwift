use sqlx::PgPool;

use crate::db::SchemaStatus;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub db: PgPool,
    /// Boot-time migration verdict (kanban t_c3823fd1). `GET /api/health` reads it so a release
    /// whose migration never applied is queryable/alertable instead of visible only in
    /// `docker logs`.
    pub schema: SchemaStatus,
    pub jwt_secret: String,
    pub internal_sync_key: String,
    pub workflowswift_url: String,
    pub coreswift_url: String,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        schema: SchemaStatus,
        jwt_secret: String,
        internal_sync_key: String,
        workflowswift_url: String,
        coreswift_url: String,
    ) -> Self {
        Self {
            db: pool.clone(),
            pool,
            schema,
            jwt_secret,
            internal_sync_key,
            workflowswift_url,
            coreswift_url,
        }
    }
}
