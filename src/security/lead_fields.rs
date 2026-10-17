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

/// `leads.name`, `first_name`, `last_name`, `email`, `company`.
pub const TEXT_CHARS: usize = 255;
/// `leads.phone`.
pub const PHONE_CHARS: usize = 50;
/// `leads.source`, `leads.stage`.
pub const SOURCE_CHARS: usize = 100;
pub const STAGE_CHARS: usize = 100;
/// `leads.status`.
pub const STATUS_CHARS: usize = 50;

/// Refuse a value the column cannot store (panel path). `None` is left to the caller.
pub fn check(field: &str, value: Option<&str>, cap: usize) -> Result<(), String> {
    match value {
        Some(v) => crate::security::display_name::check_len(v, cap, field),
        None => Ok(()),
    }
}

/// Cut a value to what the column holds, on a CHARACTER boundary (public capture path).
pub fn fit(value: &str, cap: usize) -> String {
    let trimmed = value.trim();
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
}
