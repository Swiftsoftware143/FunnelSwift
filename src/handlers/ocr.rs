use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::io::Write;

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct OcrRequest {
    pub image_base64: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize, Default)]
pub struct OcrResult {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub company: String,
    pub title: String,
    pub raw_text: String,
}

fn extract_email(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        if word.contains('@') && word.contains('.') {
            let cleaned: String = word.chars()
                .filter(|&c| c.is_ascii_alphanumeric() || c == '@' || c == '.' || c == '_' || c == '-')
                .collect();
            return Some(cleaned);
        }
    }
    None
}

fn extract_phone(text: &str) -> Option<String> {
    let cleaned: String = text.chars().filter(|&c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.' || c == '(' || c == ')' || c == ' ').collect();
    let digits: String = cleaned.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 7 && digits.len() <= 15 {
        Some(cleaned.trim().to_string())
    } else {
        None
    }
}

pub async fn handle_parse_card(
    _auth: AuthUser,
    State(_state): State<AppState>,
    Json(req): Json<OcrRequest>,
) -> AppResult<Json<OcrResult>> {
    let base64_data = req.image_base64.or(req.image).ok_or_else(|| {
        AppError::BadRequest("image_base64 field is required".to_string())
    })?;

    let image_bytes = base64::decode(&base64_data)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64: {}", e)))?;

    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("ocr_{}.png", uuid::Uuid::new_v4()));
    
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(&image_bytes)?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(AppError::Internal(format!("Failed to write temp file: {}", e)));
    }

    // Run tesseract - use shell for stderr handling
    let raw_result = std::process::Command::new("tesseract")
        .arg(temp_path.to_str().unwrap_or(""))
        .arg("stdout")
        .arg("-l")
        .arg("eng")
        .output();

    let _ = std::fs::remove_file(&temp_path);

    let raw_text = match raw_result {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        }
        Ok(_) => {
            // Tesseract failed silently - return empty
            return Ok(Json(OcrResult::default()));
        }
        Err(e) => {
            return Err(AppError::Internal(format!("Tesseract launch error: {}", e)));
        }
    };

    let raw_trimmed = raw_text.trim().to_string();
    if raw_trimmed.is_empty() {
        return Ok(Json(OcrResult::default()));
    }

    let lines: Vec<&str> = raw_trimmed.lines().collect();
    let email = extract_email(&raw_trimmed);
    let phone = extract_phone(&raw_trimmed);

    let name = if !lines.is_empty() {
        let first = lines[0].trim();
        if first.len() > 2 && first.len() < 60 && first.contains(' ') {
            first.to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let mut company = String::new();
    let mut title = String::new();
    let skip_keywords = ["email", "phone", "tel", "fax", "linkedin", "twitter", "www.", "http"];

    for &line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.len() < 3 { continue; }
        let lower = trimmed.to_lowercase();
        if skip_keywords.iter().any(|k| lower.contains(k)) { continue; }
        if trimmed == lines[0].trim() { continue; }

        if company.is_empty() {
            company = trimmed.to_string();
        } else if title.is_empty() {
            title = trimmed.to_string();
        } else {
            break;
        }
    }

    Ok(Json(OcrResult {
        name,
        email: email.unwrap_or_default(),
        phone: phone.unwrap_or_default(),
        company,
        title,
        raw_text: raw_trimmed,
    }))
}
