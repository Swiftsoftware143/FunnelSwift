use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LinkedInRequest {
    pub url: String,
}

#[derive(Serialize)]
pub struct LinkedInProfile {
    pub name: String,
    pub headline: String,
    pub company: String,
    pub title: String,
    pub location: String,
    pub profile_url: String,
    pub about: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub async fn handle_linkedin_lookup(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(req): Json<LinkedInRequest>,
) -> AppResult<Json<LinkedInProfile>> {
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::BadRequest("url is required".to_string()));
    }
    if !url.to_lowercase().contains("linkedin.com/in/") {
        return Err(AppError::BadRequest("Invalid LinkedIn URL. Must contain 'linkedin.com/in/'".to_string()));
    }

    // Call the Playwright scraper service
    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "url": url,
        "action": "linkedin_profile"
    });

    let resp = client
        .post("http://localhost:8092/scrape")
        .json(&payload)
        .timeout(std::time::Duration::from_secs(35))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Scraper service error: {}", e)))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Invalid scraper response: {}", e)))?;

    let scraper_error = body["error"].as_str().unwrap_or("").to_string();
    let data = &body["data"];

    let name = data["name"].as_str().unwrap_or("").to_string();
    let headline = data["headline"].as_str().unwrap_or("").to_string();
    let location = data["location"].as_str().unwrap_or("").to_string();
    let about = data["about"].as_str().unwrap_or("").to_string();
    let company = data["company"].as_str().unwrap_or("").to_string();
    let title = data["title"].as_str().unwrap_or("").to_string();
    let auth_wall = data["_auth_wall"].as_bool().unwrap_or(false);

    let (source, note) = if !scraper_error.is_empty() {
        ("scraper_error".to_string(), Some(scraper_error))
    } else if auth_wall {
        ("auth_wall".to_string(), Some("LinkedIn login required to view this profile. Set up LinkedIn cookies on the scraper service for authenticated scraping.".to_string()))
    } else if !name.is_empty() {
        ("scraper_live".to_string(), None)
    } else {
        ("scraper_empty".to_string(), Some("Scraper returned empty data. The profile may not exist or is not accessible.".to_string()))
    };

    Ok(Json(LinkedInProfile {
        name,
        headline,
        company,
        title,
        location,
        profile_url: url,
        about,
        source,
        note,
    }))
}
