use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{request::Parts, StatusCode},
    RequestPartsExt,
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db::Database;
use crate::error::{AppError, Result};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,  // User ID
    pub email: String,
    pub role: String,
    pub tenant_id: Option<String>,
    pub exp: usize,
    pub iat: usize,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub role: String,
    pub tenant_id: Option<String>,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    Database: axum::extract::FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self> {
        // Extract token from Authorization header
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));

        let token = auth_header.ok_or_else(|| {
            AppError::Auth("Missing or invalid Authorization header".to_string())
        })?;

        // Decode and validate JWT
        let user = validate_token(token).await?;

        Ok(user)
    }
}

pub async fn validate_token(token: &str) -> Result<AuthUser> {
    let supabase_jwt_secret = std::env::var("SUPABASE_JWT_SECRET")
        .expect("SUPABASE_JWT_SECRET must be set");

    let validation = Validation::new(Algorithm::HS256);
    let decoding_key = DecodingKey::from_secret(supabase_jwt_secret.as_bytes());

    let token_data = decode::<Claims>(token, &decoding_key, &validation)?;

    Ok(AuthUser {
        id: token_data.claims.sub,
        email: token_data.claims.email,
        role: token_data.claims.role,
        tenant_id: token_data.claims.tenant_id,
    })
}

// Alternative: Validate via Supabase API
pub async fn validate_token_via_api(token: &str) -> Result<AuthUser> {
    let supabase_url = std::env::var("SUPABASE_URL")
        .expect("SUPABASE_URL must be set");
    let supabase_anon_key = std::env::var("SUPABASE_ANON_KEY")
        .expect("SUPABASE_ANON_KEY must be set");

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/auth/v1/user", supabase_url))
        .header("Authorization", format!("Bearer {}", token))
        .header("apikey", &supabase_anon_key)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(AppError::Auth("Invalid token".to_string()));
    }

    let user_data: serde_json::Value = response.json().await?;
    
    Ok(AuthUser {
        id: user_data["id"].as_str().unwrap_or_default().to_string(),
        email: user_data["email"].as_str().unwrap_or_default().to_string(),
        role: user_data["role"].as_str().unwrap_or("user").to_string(),
        tenant_id: user_data["tenant_id"].as_str().map(|s| s.to_string()),
    })
}