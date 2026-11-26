//! Length guards for the lead text fields, at every boundary that writes them.
//!
//! `leads.name` / `first_name` / `last_name` / `email` / `company` are `varchar(255)`,
//! `phone` is `varchar(50)`, `source` and `stage` are `varchar(100)`. None of the three writers
//! checked any of them, so an over-long value reached the INSERT and came back as the raw
//! `500 {"error":"Database error"}`. Measured live 2026-10-17 on the admin workspace:
//! `POST /api/v1/leads` with a 300-character `name` -> 500, and the same with an 80-character
//! `phone` -> 500.
//!
//! Two different answers, on purpose:
//!
//! * [`check`] — the PANEL/API path. The operator is present and can fix the text, so an
//!   unstorable value is REFUSED and the field is named (the handler maps it to a 422).
//! * [`fit`] — the PUBLIC capture paths (`/api/v1/web-to-lead`, the kinetic card form), where the
//!   person filling the form is a stranger and the lead is the product. A long company name or a
//!   pasted phone number must not cost the capture, so the value is trimmed and cut to what the
//!   column holds instead of the submission failing.
//!
//! Caps are the live column widths, so nothing that used to fit is refused or shortened.
//!
//! CONTROL CHARACTERS ARE THE THIRD CASE (measured live 2026-11-26). A NUL byte is not storable in
//! a Postgres `text`/`varchar` column at all — the driver answers `invalid byte sequence for
//! encoding "UTF8": 0x00` and the handler turned that into a raw `500 {"error":"Database error"}`.
//! Reproduced on both arms the same minute: `POST /funnel/:slug/submit` with a name holding U+0000
//! -> 500 (a visitor's capture LOST, on the app's only unauthenticated write), and
//! `POST /api/v1/leads` with the same byte -> 500 (where the 255-character cap right beside it
//! already answered a clean 422). The same split as above applies: the public door STRIPS what a
//! column cannot hold and keeps the lead, the panel door REFUSES and names the field.

/// `leads.name`, `first_name`, `last_name`, `email`, `company`.
pub const TEXT_CHARS: usize = 255;
/// `leads.phone`.
pub const PHONE_CHARS: usize = 50;
/// `leads.source`, `leads.stage`.
pub const SOURCE_CHARS: usize = 100;
pub const STAGE_CHARS: usize = 100;
/// `leads.status`.
pub const STATUS_CHARS: usize = 50;

/// Characters no lead column can hold, or that never belong in one.
///
/// A NUL byte makes the statement fail outright (`invalid byte sequence for encoding "UTF8": 0x00`)
/// and surfaces as a 500; every other control character stores fine but is junk in a name, company
/// or phone number, and renders as a broken glyph in the panel and in mail.
fn without_control_chars(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).collect()
}

/// Remove the ONE character no Postgres text column can store: U+0000.
///
/// Free-text fields (`leads.notes`, the card form's `message`) keep newlines and tabs — they are
/// notes — but a NUL byte makes the whole statement fail (`invalid byte sequence for encoding
/// "UTF8": 0x00`) and the caller answered a raw 500 that lost the write. Measured live 2026-11-26 on
/// the panel's lead create/edit and on the public card form.
pub fn strip_nul(value: &str) -> String {
    value.chars().filter(|c| *c != '\u{0}').collect()
}

/// A `%…%` ILIKE pattern that matches the text the caller TYPED, not a pattern.
///
/// A search box is literal text, but `format!("%{search}%")` hands the term to `ILIKE` as a
/// PATTERN: `%` and `_` are wildcards there. Measured live 2026-11-26 on the running binary with a
/// two-lead probe workspace — `GET /api/v1/leads?search=%` returned BOTH rows and `?search=_` did
/// the same, so a search for `50%` matched every row whose text contains "50", and a search for `_`
/// matched everything: the box silently ignored what the operator typed. The same pattern was used
/// by the CSV export (a filtered table exported unfiltered rows) and by the MCP card search.
///
/// A NUL inside the term is a second, harder failure: the bound text reaches Postgres as a byte it
/// cannot decode and the list answers `500 {"error":"Database error"}` instead of rows (measured live
/// the same minute, `?search=%00`). It is stripped rather than refused — the rest of the term is
/// still the search the operator meant, and this is a READ.
pub fn like_contains(raw: &str) -> String {
    let cleaned = strip_nul(raw);
    let mut out = String::with_capacity(cleaned.len() + 2);
    out.push('%');
    for c in cleaned.chars() {
        // `#` is the escape character the matching statements declare (`ESCAPE '#'`), so no
        // backslash ever has to survive Rust's own string escaping. A literal one is escaped too,
        // otherwise a search for "Job #12" would eat the character after it.
        if matches!(c, '#' | '%' | '_') {
            out.push('#');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Refuse a value the column cannot store (panel path). `None` is left to the caller.
pub fn check(field: &str, value: Option<&str>, cap: usize) -> Result<(), String> {
    match value {
        Some(v) => {
            if v.chars().any(|c| c.is_control()) {
                return Err(format!("{field}: must not contain control characters"));
            }
            crate::security::display_name::check_len(v, cap, field)
        }
        None => Ok(()),
    }
}

/// Cut a value to what the column holds, on a CHARACTER boundary (public capture path).
pub fn fit(value: &str, cap: usize) -> String {
    // Control characters (a pasted NUL above all) are removed BEFORE the trim so the lead is still
    // captured with its visible text instead of the whole submission dying on a raw 500.
    let cleaned = without_control_chars(value);
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= cap {
        trimmed.to_string()
    } else {
        trimmed.chars().take(cap).collect()
    }
}

/// [`fit`] for a field the caller may not have sent at all.
pub fn fit_opt(value: Option<&str>, cap: usize) -> Option<String> {
    value.map(|v| fit(v, cap))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_storable_value_is_untouched() {
        assert_eq!(fit("  Ada Lovelace  ", TEXT_CHARS), "Ada Lovelace");
        assert!(check("phone", Some("+1 305 555 0100"), PHONE_CHARS).is_ok());
    }

    #[test]
    fn an_unstorable_value_is_cut_to_the_column_not_dropped() {
        let long = "é".repeat(80);
        let fitted = fit(&long, PHONE_CHARS);
        assert_eq!(fitted.chars().count(), PHONE_CHARS);
        // Cutting by CHARACTERS, not bytes: the result is still valid UTF-8 of the right width.
        assert_eq!(fitted, "é".repeat(PHONE_CHARS));
    }

    #[test]
    fn the_panel_path_names_the_field_it_refuses() {
        let err = check("company", Some(&"x".repeat(300)), TEXT_CHARS).unwrap_err();
        assert!(err.contains("company"), "{err}");
        assert!(err.contains("255"), "{err}");
    }

    #[test]
    fn a_control_character_is_stripped_from_a_public_capture_not_turned_into_a_500() {
        // Measured live 2026-11-26: a NUL byte in the submitted name reached the INSERT and the
        // driver answered `invalid byte sequence for encoding "UTF8": 0x00` -> 500, losing the lead.
        let fitted = fit("Ada\u{0} Lovelace\u{7}", TEXT_CHARS);
        assert_eq!(fitted, "Ada Lovelace");
        assert!(!fitted.chars().any(|c| c.is_control()));
        // The value is still cut to the column after the strip, so nothing new can overflow it.
        assert_eq!(
            fit(&("é\u{0}".repeat(200)), PHONE_CHARS).chars().count(),
            PHONE_CHARS
        );
    }

    #[test]
    fn the_panel_path_refuses_a_control_character_and_names_the_field() {
        let err = check("name", Some("Pane\u{0}l"), TEXT_CHARS).unwrap_err();
        assert!(err.contains("name"), "{err}");
        assert!(err.contains("control characters"), "{err}");
        // A clean value is untouched by the new arm.
        assert!(check("name", Some("Ada Lovelace"), TEXT_CHARS).is_ok());
    }

    #[test]
    fn a_search_term_is_matched_literally_not_as_a_pattern() {
        // Measured live 2026-11-26 on the running binary: `?search=%` and `?search=_` each returned
        // the WHOLE probe workspace (2 of 2 leads), i.e. the box ignored what was typed.
        assert_eq!(like_contains("50%"), "%50#%%");
        assert_eq!(like_contains("a_b"), "%a#_b%");
        // A literal escape character is escaped too, so "Job #12" does not eat the next character.
        assert_eq!(like_contains("Job #12"), "%Job ##12%");
        // A plain term is untouched, so an ordinary search keeps working exactly as before.
        assert_eq!(like_contains("Acme"), "%Acme%");
        // A NUL in the term is stripped, not passed to Postgres (measured: `?search=%00` -> 500).
        assert_eq!(like_contains("Ac\u{0}me"), "%Acme%");
        assert_eq!(like_contains("\u{0}"), "%%");
    }
}
