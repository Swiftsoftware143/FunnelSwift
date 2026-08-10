pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
}

impl Config {
    /// Read configuration from environment variables.
    /// Returns an error if JWT_SECRET is not set.
    pub fn from_env() -> Result<Self, std::env::VarError> {
        let jwt_secret = std::env::var("JWT_SECRET")?;
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://swift:swift@127.0.0.1:5432/funnelswift".into());
        Ok(Self {
            database_url,
            jwt_secret,
        })
    }
}
