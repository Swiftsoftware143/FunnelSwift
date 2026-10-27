use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo: Option<String>,
    pub colors: Option<serde_json::Value>,
    pub settings: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub name: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub tenant_id: String,
    pub email: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
    pub aud: Option<String>,
    pub iss: Option<String>,
    #[serde(default)]
    pub impersonating: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    /// OPTIONAL: the fleet's signup model is NAME + EMAIL only, and the four sibling apps mint a
    /// password when this is absent (kanban cron 2026-10-27). This door used to require it, so an
    /// integration or the native app's fallback posting the documented shape was refused 422
    /// "Enter your password." while /api/v1/auth/signup the same second answered 201. Absent or
    /// empty now mints one and mails it; a SUPPLIED password keeps the 6-character policy below.
    #[serde(default)]
    pub password: String,
    pub name: String,
    pub tenant_name: Option<String>,
    /// The plan/product the visitor picked before signing up.
    ///
    /// The public signup page forwards it as `plan` (`/signup.html?plan=kinetic-pro` — the pricing
    /// cards have carried the slug since 2026-10-21), while API callers have always used
    /// `plan_slug`. BOTH names are accepted here, because a field that only one of the two callers
    /// can fill is a field that silently does nothing: measured live 2026-10-21, the page posted
    /// `plan: "kinetic-pro"` and this struct ignored it, so `register` seated the visitor on the
    /// Capture free tier while the page had just told them they picked Kinetic.
    #[serde(default, alias = "plan")]
    pub plan_slug: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub tenant_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}
