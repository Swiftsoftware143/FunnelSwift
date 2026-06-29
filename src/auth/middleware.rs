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

pub struct AuthUser {
    pub user_id: String,
    pub tenant_id: String,
    pub email: String,
    pub role: String,
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

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(app_state.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| {
            tracing::debug!("JWT decode error: {e}");
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Invalid or expired token"})),
            )
                .into_response()
        })?;

        Ok(AuthUser {
            user_id: token_data.claims.sub,
            tenant_id: token_data.claims.tenant_id,
            email: token_data.claims.email,
            role: token_data.claims.role,
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
