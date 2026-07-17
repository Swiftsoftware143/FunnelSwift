use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde_json::json;

use crate::auth::models::Claims;
use crate::state::AppState;

const JWT_ISSUER: &str = "funnelswift";
const JWT_AUDIENCE: &str = "funnelswift-api";

pub struct AuthUser {
    pub user_id: String,
    pub tenant_id: String,
    pub email: String,
    pub role: String,
    pub is_admin: bool,
    pub token: String,
}

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Missing authorization header"})),
                )
                    .into_response()
            })?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Invalid authorization format"})),
                )
                    .into_response()
            })?;

        let mut validation = Validation::default();
        validation.set_issuer(&[JWT_ISSUER]);
        validation.set_audience(&[JWT_AUDIENCE]);
        validation.validate_exp = true;
        validation.required_spec_claims.clear();
        validation.required_spec_claims.insert("exp".to_string());

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(app_state.jwt_secret.as_bytes()),
            &validation,
        )
        .map_err(|e| {
            tracing::debug!("JWT decode error: {e}");
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Invalid or expired token"})),
            )
                .into_response()
        })?;

        let token_str = token.to_string();

        Ok(AuthUser {
            user_id: token_data.claims.sub,
            tenant_id: token_data.claims.tenant_id,
            email: token_data.claims.email,
            is_admin: token_data.claims.role == "admin",
            role: token_data.claims.role,
            token: token_str,
        })
    }
}

/// Helper trait to extract AppState from a ref (avoids cloning)
pub trait FromRef<T> {
    fn from_ref(input: &T) -> Self;
}

impl FromRef<AppState> for AppState {
    fn from_ref(input: &AppState) -> Self {
        input.clone()
    }
}
