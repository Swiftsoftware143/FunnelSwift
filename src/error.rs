use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;
pub type AppResult<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("Authentication error: {0}")]
    Auth(String),
    
    #[error("Authorization error: {0}")]
    Forbidden(String),
    
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Bad request: {0}")]
    BadRequest(String),
    
    #[error("Internal server error: {0}")]
    Internal(String),
    
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    
    #[error("Conflict: {0}")]
    Conflict(String),
    
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                let _ = std::fs::write("/tmp/funnel_error.log", format!("DB ERROR: {}\n", e));
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".to_string())
            }
            AppError::Auth(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AppError::Jwt(e) => {
                tracing::error!("JWT error: {}", e);
                (StatusCode::UNAUTHORIZED, "Invalid token".to_string())
            }
            AppError::Reqwest(e) => {
                tracing::error!("HTTP client error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "External service error".to_string())
            }
            AppError::Io(e) => {
                tracing::error!("IO error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AppError::NotImplemented(msg) => {
                tracing::warn!("Not implemented: {}", msg);
                (StatusCode::NOT_IMPLEMENTED, format!("Not implemented: {}", msg))
            }
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
        };

        let body = Json(json!({
            "error": error_message,
            "status": status.as_u16()
        }));

        (status, body).into_response()
    }
}