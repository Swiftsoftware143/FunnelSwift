use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct StoreLinkedInAuthRequest {
    pub cookies: String,
    pub expires_at: Option<String>,
}

#[derive(Serialize)]
pub struct LinkedInAuthStatus {
    pub status: String,
    pub cookie_expires_at: Option<String>,
    pub created_at: String,
}

// POST /api/v1/linkedin/auth — store LinkedIn cookies for authenticated user
pub async fn store_linkedin_auth(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<StoreLinkedInAuthRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let pool = &state.pool;
    let user_id = &auth.user_id;

    if req.cookies.trim().is_empty() {
        return Err(AppError::BadRequest("cookies cannot be empty".to_string()));
    }

    // Parse to validate they're valid JSON
    if serde_json::from_str::<serde_json::Value>(&req.cookies).is_err() {
        return Err(AppError::BadRequest("cookies must be valid JSON array".to_string()));
    }

    // Upsert: insert or update
    sqlx::query(
        r#"
        INSERT INTO linkedin_auths (user_id, encrypted_cookies, cookie_expires_at, status)
        VALUES ($1, $2, $3::timestamptz, 'active')
        ON CONFLICT (user_id)
        DO UPDATE SET
            encrypted_cookies = $2,
            cookie_expires_at = COALESCE($3::timestamptz, linkedin_auths.cookie_expires_at),
            status = 'active',
            updated_at = NOW()
        "#
    )
    .bind(user_id)
    .bind(&req.cookies)
    .bind(&req.expires_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to store LinkedIn auth: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "LinkedIn auth stored successfully"
    })))
}

// GET /api/v1/linkedin/auth/status — check LinkedIn auth status
pub async fn get_linkedin_auth_status(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<LinkedInAuthStatus>> {
    let pool = &state.pool;
    let user_id = &auth.user_id;

    let row = sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>)>(
        r#"
        SELECT status, cookie_expires_at, created_at
        FROM linkedin_auths
        WHERE user_id = $1
        "#
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to fetch LinkedIn auth: {}", e)))?;

    match row {
        Some((status, expires_at, created_at)) => {
            let now = chrono::Utc::now();
            let final_status = if status == "active" {
                if let Some(exp) = expires_at {
                    if exp < now { "expired" } else { "active" }
                } else {
                    "active"
                }
            } else {
                &status
            };

            Ok(Json(LinkedInAuthStatus {
                status: final_status.to_string(),
                cookie_expires_at: expires_at.map(|d| d.to_rfc3339()),
                created_at: created_at.to_rfc3339(),
            }))
        }
        None => Ok(Json(LinkedInAuthStatus {
            status: "disconnected".to_string(),
            cookie_expires_at: None,
            created_at: String::new(),
        })),
    }
}

// DELETE /api/v1/linkedin/auth — disconnect LinkedIn auth
pub async fn delete_linkedin_auth(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let pool = &state.pool;
    let user_id = &auth.user_id;

    let result = sqlx::query(
        "DELETE FROM linkedin_auths WHERE user_id = $1"
    )
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to delete LinkedIn auth: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "deleted": result.rows_affected() > 0
    })))
}

// GET /api/v1/linkedin/cookies/:user_id — internal endpoint for scraper to fetch cookies
pub async fn get_linkedin_cookies_for_user(
    axum::extract::Path(user_id): axum::extract::Path<uuid::Uuid>,
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<serde_json::Value>> {
    let pool = &state.pool;

    let api_key = params.get("api_key").ok_or_else(|| AppError::Unauthorized("Missing api_key".to_string()))?;
    let expected_key = std::env::var("INTERNAL_API_KEY")
        .unwrap_or_else(|_| "swift-internal-key-2026".to_string());

    if api_key != &expected_key {
        return Err(AppError::Unauthorized("Invalid api_key".to_string()));
    }

    let row = sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>)>(
        r#"
        SELECT encrypted_cookies, cookie_expires_at
        FROM linkedin_auths
        WHERE user_id = $1 AND status = 'active'
        "#
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to fetch cookies: {}", e)))?;

    match row {
        Some((cookies, expires_at)) => {
            if let Some(exp) = expires_at {
                if exp < chrono::Utc::now() {
                    return Ok(Json(serde_json::json!({
                        "found": false,
                        "error": "Cookies expired"
                    })));
                }
            }

            match serde_json::from_str::<serde_json::Value>(&cookies) {
                Ok(cookie_array) => {
                    Ok(Json(serde_json::json!({
                        "found": true,
                        "cookies": cookie_array
                    })))
                }
                Err(_) => {
                    Ok(Json(serde_json::json!({
                        "found": false,
                        "error": "Invalid cookie format in database"
                    })))
                }
            }
        }
        None => Ok(Json(serde_json::json!({
            "found": false,
            "error": "No LinkedIn auth found for this user"
        }))),
    }
}
