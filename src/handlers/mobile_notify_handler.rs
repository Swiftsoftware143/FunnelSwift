//! Public "notify me" interest capture for the mobile app (`POST /api/v1/mobile-notify`).
//!
//! # Why this exists
//!
//! The served `/download-app` page (linked from the marketing homepage twice and from `/kinetic`
//! once) told the visitor "We're building the native iOS app. Leave your email to get notified when
//! it launches" and then rendered `<a class="dl-btn gray">Coming Soon — Notify Me</a>`: no `href`,
//! no input, no backend. Measured 2026-10-16 — a real click did nothing, and the page asked for
//! something it gave the visitor no way to provide. The Android half of the same page already
//! downloads a real APK; this is the other half.
//!
//! # The contract
//!
//! * `POST /api/v1/mobile-notify` — anonymous by design. The caller is a visitor on the marketing
//!   site who owns no account, which is precisely why this route is on [`crate::auth::route_policy`]
//!   `PUBLIC_ROUTES`; the write carries no tenant column (the interest is in the PRODUCT's mobile
//!   app, not in a customer workspace), so an anonymous caller can neither read nor touch any
//!   customer's data through it.
//! * Body: `{"email": "...", "platform": "ios"|"android", "source": "<optional label>"}`.
//!   `platform` defaults to `ios` (the only ask on the page) and is refused if it is neither.
//! * `200 {"ok":true,"message":"…"}` after the row exists. Idempotent on `(platform, email)`: the
//!   UNIQUE constraint (migration 102) plus `ON CONFLICT DO NOTHING` mean a double click, a refresh
//!   or a repeated POST records one row and still gets the same confirmation — never an error for
//!   something that worked, and never a way to tell a stranger whether an address is already on the
//!   list.
//!
//! # What it deliberately does NOT do
//!
//! It does not send mail. Capturing interest must not become test traffic against the fleet's mail
//! provider, and the promise is a future notification, not an immediate one — the visitor sees the
//! confirmation on the page they are already looking at.

use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    state::AppState,
};

/// The longest address we will store — RFC 5321's path limit and the width of
/// `mobile_platform_interest.email` (varchar(254)). An over-long body is refused with a sentence
/// rather than allowed to become a database error.
const MAX_EMAIL_CHARS: usize = 254;

/// The platforms the download page can ask about. Only these two exist in the product's world:
/// Android (a shipped APK) and iOS (the one being built).
const PLATFORMS: [&str; 2] = ["ios", "android"];

/// Trim and lower-case. `A@B.com ` and `a@b.com` are one person, and the UNIQUE `(platform, email)`
/// guard only merges them if what we store is normalised. Lower-casing the domain is always correct;
/// the local part is technically case-sensitive, but no provider this fleet sends through treats it
/// so, and two rows for one person is the worse failure.
fn normalise_email(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Is this a shape we are willing to store?
///
/// Deliberately shallow: a lead list is not an identity system, and a validator stricter than the
/// mail system loses real addresses. It refuses the empty, the over-long, the whitespace-bearing,
/// the multi-`@` and the dotless-domain, and nothing else.
fn looks_like_email(email: &str) -> bool {
    if email.is_empty() || email.len() > MAX_EMAIL_CHARS || email.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// `POST /api/v1/mobile-notify` — record that somebody wants the mobile app on `platform`.
///
/// Public (see the module docs and `route_policy::PUBLIC_ROUTES`); the handler owns its own input
/// discipline because the boundary cannot: every field is length-capped, `platform` is a closed
/// set, and the only write is one idempotent INSERT.
pub async fn notify_mobile_interest(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let platform = body
        .get("platform")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "ios".to_string());
    if !PLATFORMS.contains(&platform.as_str()) {
        return Err(AppError::Validation(
            "We only have a mobile app on iOS and Android".to_string(),
        ));
    }

    let raw_email = body
        .get("email")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("An email address is required".to_string()))?;
    let email = normalise_email(raw_email);
    if !looks_like_email(&email) {
        return Err(AppError::Validation(
            "That does not look like an email address".to_string(),
        ));
    }

    // Free-form label, capped so an anonymous caller cannot store an essay. Anything missing,
    // empty or oversized falls back to the page this route was built for.
    let source = body
        .get("source")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .unwrap_or_else(|| "download-app".to_string());

    sqlx::query(
        "INSERT INTO mobile_platform_interest (platform, email, source)
         VALUES ($1, $2, $3)
         ON CONFLICT (platform, email) DO NOTHING",
    )
    .bind(&platform)
    .bind(&email)
    .bind(&source)
    .execute(&state.pool)
    .await?;

    // The SAME answer whether the row was new or already there: the visitor's outcome is identical
    // (they are on the list), and the response must not become a way to probe the list.
    Ok((
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "message": "Thanks \u{2014} we will let you know the moment the mobile app is ready."
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::{looks_like_email, normalise_email};

    #[test]
    fn an_address_is_normalised_before_it_is_stored() {
        assert_eq!(normalise_email("  Ada@Example.COM "), "ada@example.com");
    }

    #[test]
    fn the_email_guard_accepts_the_shapes_a_person_actually_types() {
        for ok in [
            "a@b.co",
            "first.last+tag@sub.domain.example.com",
            "x1@y-z.io",
        ] {
            assert!(looks_like_email(ok), "{ok} must be accepted");
        }
    }

    #[test]
    fn the_email_guard_refuses_only_the_impossible() {
        for bad in [
            "",
            "no-at-sign",
            "@nolocal.com",
            "nodomain@",
            "two@at@signs.com",
            "spaces in@address.com",
            "trailingdot@example.",
            "leadingdot@.com",
        ] {
            assert!(!looks_like_email(bad), "{bad} must be refused");
        }
        // The ceiling is the column width, not a preference.
        let long = format!("{}@example.com", "a".repeat(300));
        assert!(!looks_like_email(&long));
    }
}
