pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://swift:swift@127.0.0.1:5432/funnelswift".into()),
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "funnelswift-jwt-secret-change-in-production-2026".into()),
        }
    }
}
