//! Display-name length validation, in ONE place, at every boundary that writes a person's name.
//!
//! Every name-bearing column in this database is `varchar(255)` — `users.name`, `users.username`,
//! `users.company`, `tenants.name`, `tenants.slug` — and the public signup DERIVES two of them from
//! the typed name (`tenants.name = "<name>'s Workspace"`, `tenants.slug = "<name>-<8 hex>"`). Before
//! this module there was no length check anywhere in the request path, so a name the DB could not
//! store reached the INSERT and came back as the raw `sqlx` error:
//!
//!   POST /api/v1/auth/signup  {"name":"<254 chars>","email":"…"}  ->  500 {"error":"Database error"}
//!
//! Measured live 2026-10-16: 200 characters succeeded, 254 failed. The derived workspace name is
//! `254 + 13 = 267 > 255`, so the FIRST statement of the signup transaction was the one refused —
//! the customer could not create an account at all and saw a server error, with no hint that their
//! name was the problem. The form carried no `maxlength` either, so a pasted paragraph was a
//! perfectly ordinary way to hit it.
//!
//! The cap is the STORAGE limit, not a product decision: 255 minus the widest suffix this app
//! appends to a name (`"'s Workspace"`, 12 characters — hence 243). Anything that fit before still
//! fits, and nothing that used to work is now refused. Call it BEFORE the write, exactly like
//! [`crate::security::email_addr::normalize`], and let the handler map the reason to a 422.

/// `users.name` / `tenants.name` / `tenants.slug` are all `varchar(255)`.
const COLUMN_CHARS: usize = 255;

/// The suffix the public signup appends to a name to name the workspace (`"{}'s Workspace"`).
const WORKSPACE_SUFFIX_CHARS: usize = "'s Workspace".len();

/// Longest name whose DERIVED workspace name still fits its column.
pub const MAX_NAME_CHARS: usize = COLUMN_CHARS - WORKSPACE_SUFFIX_CHARS;

/// Normalise a person's display name for STORAGE, or explain why it cannot be stored.
///
/// Trims, requires a non-empty value, refuses one longer than [`MAX_NAME_CHARS`], and refuses
/// control characters (a name is rendered into HTML and mail subjects). Returns the value to
/// persist — the caller must bind THIS, never the raw input.
pub fn normalize(raw: &str) -> Result<String, String> {
    normalize_capped(raw, MAX_NAME_CHARS, "name")
}

/// The same guard for the other name-bearing fields a profile or a signup writes (`username`,
/// `company`, and the register path's explicit `tenant_name`).
pub fn normalize_capped(raw: &str, max_chars: usize, label: &str) -> Result<String, String> {
    let value = raw.trim();

    if value.is_empty() {
        return Err(format!("{label}: is required"));
    }
    // Counted in CHARACTERS, which is what `varchar(n)` measures: a name of 200 accented letters
    // is 200 characters (400 bytes) and fits a `varchar(255)` column.
    if value.chars().count() > max_chars {
        return Err(format!("{label}: is longer than {max_chars} characters"));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("{label}: must not contain control characters"));
    }

    Ok(value.to_string())
}

/// Enforce ONLY the storage limit, for a `varchar(255)` field that may legitimately be EMPTY
/// (`username` and `company` are clearable from the profile screen, and the register path's
/// `tenant_name` is optional). A blank value is the caller's business; a value the column cannot
/// hold is not, and used to surface as a raw 500 `"Database error"`.
pub fn check_len(raw: &str, max_chars: usize, label: &str) -> Result<(), String> {
    if raw.trim().chars().count() > max_chars {
        return Err(format!("{label}: is longer than {max_chars} characters"));
    }
    Ok(())
}

/// The ONE shape of `tenants.slug`, shared by every provisioning site.
///
/// Lowercase, spaces to dashes, and then ONLY `[a-z0-9_-]` survives. The last step is the one that
/// matters: two writers (`auth::handlers::register` and `handlers::public_signup_handler`) built the
/// slug by hand from a user-supplied name, so an apostrophe or an accented letter landed in the
/// identifier — measured 2026-10-17, `"Zed Naming's Workspace"` produced the slug
/// `zed-naming's-workspace-516ad5b6`, which is not a URL-safe identifier and no slug lookup can be
/// written against it. `tenant_handler::slugify` already applied this filter; this is that function,
/// moved where every writer can reach it.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for c in name.trim().to_lowercase().chars() {
        let c = if c == ' ' { '-' } else { c };
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            last_was_dash = false;
        } else if c == '-' && !last_was_dash && !out.is_empty() {
            // Squeezed and never leading: dropping an accented first letter must not leave the
            // identifier starting with a dash (`Ó Súilleabháin` -> `silleabhin`, not `-silleabhin`).
            out.push('-');
            last_was_dash = true;
        }
        // Anything else (an apostrophe, an accented letter) is DROPPED, not transliterated.
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_derived_workspace_name_always_fits_its_column() {
        // The invariant this module exists for: a name at the cap, plus the suffix the signup
        // appends, is still storable.
        let name = "n".repeat(MAX_NAME_CHARS);
        assert!(normalize(&name).is_ok());
        assert!(format!("{name}'s Workspace").chars().count() <= COLUMN_CHARS);
    }

    #[test]
    fn refuses_the_length_that_produced_a_500() {
        // Measured live 2026-10-16: 254 characters reached the INSERT and answered
        // `500 {"error":"Database error"}`. It must now be a caller-safe 422 reason.
        let too_long = "n".repeat(254);
        assert_eq!(
            normalize(&too_long),
            Err(format!("name: is longer than {MAX_NAME_CHARS} characters"))
        );
        // …and one character under the cap still passes, so the guard does not shrink the contract.
        assert!(normalize(&"n".repeat(MAX_NAME_CHARS)).is_ok());
    }

    #[test]
    fn accepts_ordinary_and_unicode_names() {
        assert_eq!(normalize("  Ada Lovelace  ").unwrap(), "Ada Lovelace");
        assert_eq!(normalize("Ó Súilleabháin").unwrap(), "Ó Súilleabháin");
        // 200 accented characters are 200 chars / 400 bytes — valid for varchar(255).
        let accented = "é".repeat(200);
        assert!(normalize(&accented).is_ok());
    }

    #[test]
    fn rejects_empty_and_control_characters() {
        assert_eq!(normalize("   "), Err("name: is required".to_string()));
        assert_eq!(normalize(""), Err("name: is required".to_string()));
        assert!(normalize("Ada\nLovelace").is_err());
    }

    #[test]
    fn a_slug_is_an_identifier_not_a_sentence() {
        // Measured live 2026-10-17: the register door built the slug by hand and produced
        // `zed-naming's-workspace-516ad5b6` — an apostrophe in a UNIQUE identifier that no slug
        // lookup or URL can address. The shared helper drops everything outside `[a-z0-9_-]`.
        assert_eq!(slugify("Zed Naming's Workspace"), "zed-namings-workspace");
        assert_eq!(slugify("  Acme Roofing  "), "acme-roofing");
        assert_eq!(slugify("Ó Súilleabháin Ltd"), "silleabhin-ltd");
        // A name that is ALREADY a clean slug is unchanged, so existing shapes do not move.
        assert_eq!(
            slugify("swiftsoftware-admin-2026"),
            "swiftsoftware-admin-2026"
        );
    }

    #[test]
    fn the_capped_arm_names_its_own_field() {
        assert_eq!(
            normalize_capped(&"x".repeat(256), COLUMN_CHARS, "company"),
            Err("company: is longer than 255 characters".to_string())
        );
        assert!(normalize_capped("Acme Ltd", COLUMN_CHARS, "company").is_ok());
        assert!(normalize_capped("", COLUMN_CHARS, "username").is_err());
    }
}
