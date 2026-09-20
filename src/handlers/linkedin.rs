use crate::error::AppResult;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde_json::{json, Value};

/// Best-effort display name derived **from the URL alone** — `jane-doe-1a2b3c` -> "Jane Doe".
///
/// This performs no network call and no scraping: LinkedIn has no public profile-search API,
/// scraping is against their ToS, and every paid enrichment provider is a commercial decision.
/// So the endpoint returns what the URL honestly supports and states its provenance in `source`.
fn name_from_slug(slug: &str) -> String {
    let mut parts: Vec<&str> = slug.split('-').filter(|p| !p.is_empty()).collect();
    // LinkedIn appends a disambiguator to a slug when the pretty form is taken: either all
    // digits, or a short hex-ish blob. Drop it so we do not put it in the contact's name.
    if let Some(last) = parts.last() {
        let all_digits = last.chars().all(|c| c.is_ascii_digit());
        let hexish = last.len() >= 6 && last.chars().all(|c| c.is_ascii_hexdigit());
        if all_digits || hexish {
            parts.pop();
        }
    }
    parts
        .iter()
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// POST /api/v1/leads/linkedin-lookup
///
/// The mobile client (`lib/http.ts` -> `linkedinLookup`) posts `{"url": "<profile url>"}` and
/// reads back `name` / `email` / `email_guess` / `company` / `headline` / `title` / `summary`.
/// This handler previously read `payload["name"]` — the one field the client never sends — and
/// returned `full_name`/`headline`, so the app's `if (!result.name && !result.email)` test
/// always failed and every grab showed "Could not extract contact information". The response
/// shape now matches what the client reads, with the original keys kept for compatibility.
pub async fn handle_linkedin_lookup(
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Json<Value>> {
    let url = payload["url"]
        .as_str()
        .or_else(|| payload["profile_url"].as_str())
        .or_else(|| payload["name"].as_str()) // legacy callers that posted a bare string
        .unwrap_or("")
        .trim()
        .to_string();

    let too_short = url.len() < 12;
    let upper = url.to_lowercase();
    let is_profile = upper.contains("linkedin.com/in/");
    if too_short || !is_profile {
        return Ok(Json(json!({
            "name": "", "full_name": "", "email": "", "email_guess": "",
            "phone": "", "company": "", "headline": "", "title": "", "summary": "",
            "profile_url": url, "found": false, "source": "linkedin-url",
            "error": "not a LinkedIn profile URL (expected https://linkedin.com/in/<slug>)",
        })));
    }

    let slug = url
        .split("/in/")
        .nth(1)
        .unwrap_or("")
        .split(['?', '#', '/'])
        .next()
        .unwrap_or("")
        .trim();
    let name = name_from_slug(slug);

    Ok(Json(json!({
        // keys the mobile client reads
        "name": name,
        "email": "",
        "email_guess": "",
        "phone": "",
        "company": "",
        "title": "",
        // the profile URL rides on `summary`, which the client passes through to the lead, so a
        // grabbed profile is never lost even though no provider fills the other fields
        "summary": format!("LinkedIn: {url}"),
        // original keys, kept so any other caller keeps working
        "full_name": name,
        "headline": "",
        "profile_url": url,
        "found": !name.is_empty(),
        "source": "linkedin-url-slug",
        "note": "Name derived from the profile URL. No external lookup provider is configured; \
                 email/phone/company require a licensed enrichment API.",
    })))
}

#[cfg(test)]
mod tests {
    use super::name_from_slug;

    #[test]
    fn derives_a_readable_name_and_drops_the_disambiguator() {
        assert_eq!(name_from_slug("jane-doe-1a2b3c"), "Jane Doe");
        assert_eq!(name_from_slug("john-q-public-123456789"), "John Q Public");
        assert_eq!(name_from_slug("madonna"), "Madonna");
        assert_eq!(name_from_slug(""), "");
    }
}
