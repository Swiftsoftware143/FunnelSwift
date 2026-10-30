//! Business-card OCR handler.
//!
//! The mobile client (`FunnelSwift-Mobile/lib/http.ts` -> `ocrParseCard`) posts
//! `{ "image_base64": "<compressed JPEG/PNG as base64>" }` to
//! `POST /api/v1/ocr/parse-card` and renders the frozen response contract
//! `{ name, title, company, email, phone, parsed }` (all strings except
//! `parsed`). Additive keys are fine; renames/removals are not.
//!
//! The actual OCR runs on the HOST (Tesseract 5.x is not in the app image and
//! the image is a live container), behind the docker-bridge service
//! `funnelswift-ocr.service` on 172.17.0.1:8093 — the same pattern as
//! `rust-compiler-guard.service` on :8092. If that service is unreachable, or
//! returns junk, this handler must NOT 500: it returns the normal empty-field
//! response with `parsed: false` so the app's existing "fill in manually"
//! fallback still works.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::features;
use crate::state::AppState;
use axum::{extract::State, Json};
use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;
use uuid::Uuid;

/// Default docker-bridge address of `funnelswift-ocr.service`.
const DEFAULT_OCR_SERVICE_URL: &str = "http://172.17.0.1:8093";
/// Slightly longer than the service's own 20s Tesseract timeout.
const OCR_TIMEOUT_SECS: u64 = 25;

/// The client only ever sends `image_base64`.
#[derive(Debug, Deserialize)]
pub struct ParseCardRequest {
    #[serde(default)]
    pub image_base64: Option<String>,
}

/// The biggest card photo the scanner accepts, in BYTES of the ORIGINAL file.
///
/// The customer app promises 12 MB in its own words ("Choose a photo of the card (up to 12 MB)")
/// and refuses anything larger in the picker, so 12 MB is the number the SERVER has to honour —
/// not axum's 2 MB default envelope.
///
/// MEASURED 2026-10-30 on the live origin, before this constant existed: a 3 MB photo (4 MB of
/// base64) answered `413 {"error":"That file is too large to upload. The maximum size is 2 MB
/// ..."}`, and `GET /api/v1/ocr/scans` stayed at 0 rows — so a phone photo, the only thing a card
/// scanner is ever fed, never reached this handler. The route carried no `DefaultBodyLimit`, so
/// axum's default applied while the page promised three times as much.
pub const MAX_PHOTO_BYTES: usize = 12 * 1024 * 1024;

/// The request envelope `POST /api/v1/ocr/parse-card` needs to carry the largest allowed photo.
///
/// Base64 inflates by 4/3, so a 12 MB photo arrives as 16 MB of JSON string. The envelope is set
/// well above that on purpose: the handler's own guard (`MAX_PHOTO_BYTES`) must be the thing that
/// answers a too-big picture with the app's 12 MB sentence, and it only runs once the body has been
/// read — so a merely-too-big photo (13-15 MB) must still fit here, or the middleware's generic
/// 2 MB envelope sentence is the one the customer reads. Applied to that ONE route so no other
/// endpoint inherits a 20 MB read limit.
pub const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;

/// The route this ceiling belongs to. The body-read deadline middleware (`src/body_deadline.rs`)
/// has to recognise it by path, because that middleware buffers the body BEFORE the router applies
/// the route's own `DefaultBodyLimit` — see the comment there.
pub const PATH: &str = "/api/v1/ocr/parse-card";

#[derive(Debug, Serialize)]
struct OcrServiceRequest<'a> {
    image_base64: &'a str,
}

#[derive(Debug, Deserialize)]
struct OcrServiceResponse {
    #[serde(default)]
    text: Option<String>,
}

// ─────────────────────────── text extraction ────────────────────────────────

/// Fields extracted from raw OCR text. All plain strings; empty == not found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CardFields {
    pub name: String,
    pub title: String,
    pub company: String,
    pub email: String,
    pub phone: String,
}

const EMAIL_PATTERN: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}";
/// Multi-format phone: optional +country, spaces / dots / dashes / parens.
const PHONE_PATTERN: &str = r"[+]?[(]?[0-9][0-9 ().-]{5,}[0-9]";
/// A bare date must not be mistaken for a phone number.
const DATE_PATTERN: &str = r"^[0-9]{4}[-/.][0-9]{1,2}[-/.][0-9]{1,2}$";

/// Legal-entity suffixes that mark a company line.
const LEGAL_SUFFIXES: &[&str] = &[
    "inc",
    "llc",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "gmbh",
    "co",
    "company",
    "llp",
    "plc",
    "pte",
    "pty",
    "bv",
    "ag",
    "sa",
    "srl",
    "oy",
    "ab",
    "group",
    "holdings",
];

/// Title vocabulary — matched as whole words inside a line.
const TITLE_WORDS: &[&str] = &[
    "ceo",
    "cto",
    "cfo",
    "coo",
    "cmo",
    "cio",
    "chief",
    "founder",
    "cofounder",
    "owner",
    "president",
    "vice",
    "director",
    "manager",
    "engineer",
    "head",
    "partner",
    "principal",
    "consultant",
    "architect",
    "designer",
    "developer",
    "lead",
    "specialist",
    "analyst",
    "accountant",
    "supervisor",
    "executive",
    "officer",
    "coordinator",
    "administrator",
    "strategist",
    "advisor",
    "counsel",
    "technician",
    "producer",
    "editor",
];

/// Multi-word title phrases (substring match on the lowercased line).
const TITLE_PHRASES: &[&str] = &[
    "head of",
    "vice president",
    "managing director",
    "co-founder",
];

fn cached<'a>(cell: &'a OnceLock<Option<Regex>>, pattern: &str) -> Option<&'a Regex> {
    cell.get_or_init(|| Regex::new(pattern).ok()).as_ref()
}

fn email_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    cached(&RE, EMAIL_PATTERN)
}

fn phone_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    cached(&RE, PHONE_PATTERN)
}

fn date_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    cached(&RE, DATE_PATTERN)
}

/// Split a line into lowercased alphanumeric tokens (punctuation dropped).
fn tokens(line: &str) -> Vec<String> {
    line.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Trim OCR noise, collapse whitespace, and drop lines without real content.
fn clean_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.replace('\t', " "))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|l| {
            l.trim_matches(|c: char| {
                !c.is_alphanumeric() && !matches!(c, '@' | '+' | '(' | ')' | '.')
            })
            .to_string()
        })
        .filter(|l| l.chars().filter(|c| c.is_alphanumeric()).count() >= 2)
        .collect()
}

fn find_email(lines: &[String]) -> String {
    let Some(re) = email_re() else {
        return String::new();
    };
    for line in lines {
        if let Some(m) = re.find(line) {
            return m.as_str().trim_end_matches(['.', ',', ';']).to_string();
        }
    }
    String::new()
}

fn find_phone(lines: &[String]) -> String {
    let (Some(re), Some(dre)) = (phone_re(), date_re()) else {
        return String::new();
    };
    for line in lines {
        if line.contains('@') {
            continue; // don't read digits out of an email address
        }
        for m in re.find_iter(line) {
            let candidate = m
                .as_str()
                .trim_matches(|c: char| matches!(c, '(' | ')' | '.' | '-' | ' '));
            if candidate.is_empty() || dre.is_match(candidate) {
                continue;
            }
            let digits = candidate.chars().filter(|c| c.is_ascii_digit()).count();
            if (7..=15).contains(&digits) {
                return candidate.to_string();
            }
        }
    }
    String::new()
}

fn has_legal_suffix(line: &str) -> bool {
    tokens(line)
        .iter()
        .any(|t| LEGAL_SUFFIXES.contains(&t.as_str()))
}

fn find_company(lines: &[String]) -> String {
    lines
        .iter()
        .find(|l| !l.contains('@') && has_legal_suffix(l))
        .cloned()
        .unwrap_or_default()
}

fn looks_like_title(line: &str) -> bool {
    if line.contains('@') {
        return false;
    }
    let lower = line.to_lowercase();
    if TITLE_PHRASES.iter().any(|p| lower.contains(p)) {
        return true;
    }
    tokens(line)
        .iter()
        .any(|t| TITLE_WORDS.contains(&t.as_str()))
}

fn find_title(lines: &[String]) -> String {
    lines
        .iter()
        .find(|l| looks_like_title(l))
        .cloned()
        .unwrap_or_default()
}

/// A person's name: 2-4 words, each alphabetic and capitalised, no digits,
/// no '@', and not a title/company line.
fn looks_like_person_name(line: &str) -> bool {
    if line.contains('@') || line.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if !(2..=4).contains(&words.len()) {
        return false;
    }
    words.iter().all(|w| {
        let core = w.trim_matches(|c: char| !c.is_alphabetic() && c != '\'' && c != '-');
        let mut chars = core.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        first.is_uppercase() && chars.all(|c| c.is_alphabetic() || c == '\'' || c == '-')
    })
}

fn find_name(lines: &[String], company: &str, title: &str) -> String {
    lines
        .iter()
        .find(|l| {
            **l != *company
                && **l != *title
                && !has_legal_suffix(l)
                && !looks_like_title(l)
                && looks_like_person_name(l)
        })
        .cloned()
        .unwrap_or_default()
}

/// Deterministic, honest extraction from raw OCR text.
pub fn extract_card_fields(text: &str) -> CardFields {
    let lines = clean_lines(text);
    let email = find_email(&lines);
    let phone = find_phone(&lines);
    let company = find_company(&lines);
    let title = find_title(&lines);
    let name = find_name(&lines, &company, &title);
    CardFields {
        name,
        title,
        company,
        email,
        phone,
    }
}

/// The frozen response contract the mobile app reads.
fn card_response(fields: &CardFields) -> Value {
    let parsed = !(fields.name.is_empty() && fields.email.is_empty() && fields.phone.is_empty());
    json!({
        "name": fields.name,
        "title": fields.title,
        "company": fields.company,
        "email": fields.email,
        "phone": fields.phone,
        "parsed": parsed,
    })
}

/// Response used whenever OCR could not produce usable fields — the app's
/// "fill in manually" fallback path. Never a 500.
fn empty_card_response() -> Value {
    card_response(&CardFields::default())
}

// ───────────────────────────── host service ─────────────────────────────────

fn service_url() -> String {
    std::env::var("OCR_SERVICE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_OCR_SERVICE_URL.to_string())
}

fn service_token() -> Option<String> {
    std::env::var("OCR_SERVICE_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// POST the base64 image to the host Tesseract service. Errors are returned as
/// plain strings so the caller can log a reason and degrade gracefully.
async fn call_ocr_service(image_base64: &str) -> Result<String, String> {
    let Some(token) = service_token() else {
        return Err("OCR_SERVICE_TOKEN is not configured in the container env".to_string());
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(OCR_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;
    let response = client
        .post(format!("{}/ocr", service_url().trim_end_matches('/')))
        .header("X-OCR-Token", token)
        .json(&OcrServiceRequest { image_base64 })
        .send()
        .await
        .map_err(|e| format!("ocr service unreachable: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("ocr service returned status {status}"));
    }
    let body: OcrServiceResponse = response
        .json()
        .await
        .map_err(|e| format!("ocr service sent unreadable body: {e}"))?;
    Ok(body.text.unwrap_or_default())
}

// ─────────────────────────────── handler ────────────────────────────────────

pub async fn handle_parse_card(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ParseCardRequest>,
) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant_id in token".into()))?;

    // `max_ocr_scans` is a SOLD metered feature; check the allowance first so an
    // over-limit plan gets the app's normal upgrade response (402).
    features::enforce_feature_limit(&state, tenant_id, "max_ocr_scans", "Card scans").await?;

    let image_base64 = payload.image_base64.unwrap_or_default();
    // The server-side twin of the picker's own 12 MB check. The route's envelope below bounds the
    // BASE64 length; this states the ceiling in the units the customer was shown, so an oversized
    // picture is refused with a sentence rather than with the middleware's generic 2 MB numbers.
    // Every refusal here happens BEFORE the metering INSERT, so an oversized picture costs the
    // workspace nothing.
    if image_base64.trim().len() / 4 * 3 > MAX_PHOTO_BYTES {
        return Err(AppError::PayloadTooLarge(format!(
            "That picture is over {} MB — choose a smaller one.",
            MAX_PHOTO_BYTES / (1024 * 1024)
        )));
    }
    if image_base64.trim().is_empty() {
        tracing::warn!(%tenant_id, "parse-card called with empty image_base64");
        return Ok(Json(empty_card_response()));
    }

    // Meter the attempt before it leaves the box, so the counter reflects the
    // work asked of the host service (see 045_create_ocr_scans.sql). The row's id
    // is kept so the reading can be written onto the SAME row (migration 120) —
    // the customer's allowance and the list behind it then come from one place.
    let scan_id: Option<Uuid> =
        match sqlx::query_scalar("INSERT INTO ocr_scans (tenant_id) VALUES ($1) RETURNING id")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await
        {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::error!(%tenant_id, "failed to record ocr_scan usage: {e}");
                None
            }
        };

    match call_ocr_service(&image_base64).await {
        Ok(text) => {
            let fields = extract_card_fields(&text);
            tracing::info!(
                %tenant_id,
                chars = text.chars().count(),
                "parse-card ocr ok"
            );
            // `ocr_ok` means a READING came back, not merely that the service answered: the host
            // can return an empty/garbage page and `extract_card_fields` then yields nothing. This
            // is the same predicate `card_response` publishes as `parsed`, so the list the customer
            // sees and the `parsed` flag they were shown can never disagree.
            let parsed =
                !(fields.name.is_empty() && fields.email.is_empty() && fields.phone.is_empty());
            record_scan_outcome(&state, scan_id, &fields, parsed).await;
            Ok(Json(card_response(&fields)))
        }
        Err(reason) => {
            tracing::warn!(%tenant_id, "parse-card degraded to manual entry: {reason}");
            record_scan_outcome(&state, scan_id, &CardFields::default(), false).await;
            Ok(Json(empty_card_response()))
        }
    }
}

/// Write what a scan produced onto the metered row it already created.
///
/// A failure here is logged and swallowed: the attempt is metered either way, and the scan itself
/// has already succeeded or degraded. Never a 500 — the same rule as the OCR call above.
async fn record_scan_outcome(
    state: &AppState,
    scan_id: Option<Uuid>,
    fields: &CardFields,
    ok: bool,
) {
    let Some(id) = scan_id else { return };
    let payload = card_response(fields);
    if let Err(e) = sqlx::query("UPDATE ocr_scans SET fields = $2, ocr_ok = $3 WHERE id = $1")
        .bind(id)
        .bind(sqlx::types::Json(payload))
        .bind(ok)
        .execute(&state.pool)
        .await
    {
        tracing::error!(scan_id = %id, "failed to record ocr_scan outcome: {e}");
    }
}

/// `GET /api/v1/ocr/scans` — the caller's own card-scan history, newest first.
///
/// Tenant-scoped by `auth.tenant_id` with no path id to spoof, so a workspace sees only its own
/// scans. Bounded to the last 20: the screen shows a recent list, and `GET /api/v1/me/usage`
/// remains the single source of truth for the allowance itself.
pub async fn list_scans(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Value>> {
    let tenant_id = Uuid::parse_str(&auth.tenant_id)
        .map_err(|_| AppError::BadRequest("Invalid tenant_id in token".into()))?;
    let rows: Vec<(
        Uuid,
        chrono::DateTime<chrono::Utc>,
        bool,
        Option<sqlx::types::Json<Value>>,
    )> = sqlx::query_as(
        "SELECT id, created_at, ocr_ok, fields FROM ocr_scans \
             WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 20",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let scans: Vec<Value> = rows
        .into_iter()
        .map(|(id, created_at, ok, fields)| {
            let reading = fields
                .map(|sqlx::types::Json(v)| v)
                .unwrap_or_else(|| json!({}));
            json!({
                "id": id,
                "created_at": created_at.to_rfc3339(),
                "ocr_ok": ok,
                "fields": reading,
            })
        })
        .collect();

    Ok(Json(json!({ "scans": scans })))
}

// ──────────────────────────────── tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Literal output captured from the live host Tesseract service for a
    /// rendered business card (Amara Okafor / Northwind Technologies Ltd).
    const REAL_CARD_TEXT: &str = "Amara Okafor\nDirector of Engineering\nNorthwind Technologies Ltd\namara.okafor@northwind.co.uk\n\n+44 20 7946 0958\n";

    fn digits(s: &str) -> String {
        s.chars().filter(|c| c.is_ascii_digit()).collect()
    }

    #[test]
    fn extracts_all_fields_from_real_card_text() {
        let f = extract_card_fields(REAL_CARD_TEXT);
        assert_eq!(f.name, "Amara Okafor");
        assert_eq!(f.title, "Director of Engineering");
        assert_eq!(f.company, "Northwind Technologies Ltd");
        assert_eq!(f.email, "amara.okafor@northwind.co.uk");
        assert_eq!(digits(&f.phone), "442079460958");
    }

    #[test]
    fn full_response_shape_is_frozen_and_parsed_true() {
        let body = card_response(&extract_card_fields(REAL_CARD_TEXT));
        assert_eq!(body["parsed"], json!(true));
        for key in ["name", "title", "company", "email", "phone"] {
            assert!(body[key].is_string(), "{key} must be a string");
        }
        assert!(body["title"].is_string());
    }

    #[test]
    fn messy_ocr_noise_still_extracts() {
        let text = "| J. RANDALL PIKE |\n~~ Vice President, Sales ~~\nAcme Robotics Inc\nrandall.pike@acme-robotics.com\nT: (415) 555-0142   C: 415.555.0199\n";
        let f = extract_card_fields(text);
        assert_eq!(f.name, "J. RANDALL PIKE");
        assert_eq!(f.title, "Vice President, Sales");
        assert_eq!(f.company, "Acme Robotics Inc");
        assert_eq!(f.email, "randall.pike@acme-robotics.com");
        assert_eq!(digits(&f.phone), "4155550142");
    }

    #[test]
    fn empty_text_parses_to_false_without_panicking() {
        let body = card_response(&extract_card_fields("   \n\n,,,\n"));
        assert_eq!(body["parsed"], json!(false));
        assert_eq!(body["name"], json!(""));
        assert_eq!(body["email"], json!(""));
    }

    #[test]
    fn email_only_card_is_parsed() {
        let body = card_response(&extract_card_fields("Reach me at hello@example.com\n"));
        assert_eq!(body["parsed"], json!(true));
        assert_eq!(body["email"], json!("hello@example.com"));
        assert_eq!(body["name"], json!(""));
    }

    #[test]
    fn title_or_company_line_is_never_mistaken_for_a_name() {
        let f = extract_card_fields("Engineering Director\nGlobex Ltd\n");
        assert_eq!(f.name, "");
        assert_eq!(f.company, "Globex Ltd");
        assert_eq!(f.title, "Engineering Director");
    }

    #[test]
    fn dates_are_not_phones() {
        let f = extract_card_fields("Acme Group\n2024-01-15\n");
        assert_eq!(f.phone, "");
    }
}
