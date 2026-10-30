//! Email-address syntax validation, in ONE place, at every boundary that writes `users.email`.
//!
//! `users.email` is an account's login identity AND the only address its credentials can ever be
//! mailed to. Before this module FunnelSwift had no format check anywhere in the request path and
//! the column had no `CHECK`, so `POST /api/v1/auth/register` accepted the literal string `bad` and
//! minted a real account whose login was not an address at all — permanently unreachable (no
//! welcome/credentials mail can ever be delivered to it), and `POST /api/v1/auth/signup` accepted
//! anything containing an `@` (`bad@`, `@`, `a b@x`). That is the defect this module closes
//! (kanban t_38017305, class census in /opt/swift/audits/t_4722a331/CLASS-CENSUS.md; the reference
//! shape was proven end-to-end on missedcallrespondr under t_54b1ffab).
//!
//! Three writers need the same answer, and they must not drift apart:
//!   * the public signup path (`auth::handlers::register`) — `POST /api/v1/auth/register`,
//!   * the other public signup path (`handlers::public_signup_handler::public_signup`) —
//!     `POST /api/v1/auth/signup`,
//!   * the admin / cross-app create path (`handlers::admin_handler::portfolio_sync`),
//!   * plus the read-only `forgot-password`/`login` lookups, which must match the same value.
//!
//! Deliberately **syntax only**: trimming and lowercasing are the normalisations this fleet already
//! ships, and nothing here tightens what an address may *mean*. Plus-aliases (`a+b@x.com`), dotted
//! locals (`a.b@x.com`) and IDN domains (`user@münchen.de`) stay valid. The mirror `CHECK` on the
//! column is deliberately LOOSER than this function (`migrations/082_users_email_format_check.sql`)
//! so the database can never refuse a value the application accepted.
//!
//! No `regex` crate in the dependency graph, and none is added: the checks are simple scans.

/// RFC 5321 forward-path limit — every real mailbox fits.
const MAX_ADDRESS_LEN: usize = 254;

/// Normalise an address for STORAGE and reject anything that is not syntactically an address.
///
/// Returns the trimmed, lowercased value the caller must persist and use in messages, or a
/// caller-safe reason (the handlers map it to `422`). Call this BEFORE any INSERT/UPDATE — never
/// after, and never let an unvalidated value reach the statement.
pub fn normalize(raw: &str) -> Result<String, String> {
    let value = raw.trim().to_lowercase();

    if value.is_empty() {
        return Err("email: is required".into());
    }
    if value.len() > MAX_ADDRESS_LEN {
        return Err(format!(
            "email: is longer than {MAX_ADDRESS_LEN} characters"
        ));
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("email: must not contain whitespace or control characters".into());
    }

    let Some((local, domain)) = value.split_once('@') else {
        return Err("email: must look like name@example.com".into());
    };
    if domain.contains('@') {
        return Err("email: must contain exactly one @".into());
    }
    if local.is_empty() {
        return Err("email: is missing the part before @".into());
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return Err("email: has an empty dot-separated part before @".into());
    }
    if domain.is_empty() {
        return Err("email: is missing the domain after @".into());
    }
    if !domain.contains('.') {
        return Err("email: domain must contain a dot (e.g. example.com)".into());
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.split('.').any(|l| l.is_empty()) {
        return Err("email: domain has an empty dot-separated part".into());
    }

    // Shapes that pass every rule above but can never receive mail. A domain LABEL may not start or
    // end with a hyphen (RFC 1035 hostname syntax), a bracketed literal (`user@[1.2.3.4]`) is not a
    // name the provider will accept, and a quoted local part (`"ab"@x.com`) is a form nothing in this
    // fleet parses — the address would be stored verbatim and every message to it would bounce. That
    // is the same dead-account class this module exists to stop (`users.email` is the ONLY address a
    // login can be mailed to). Measured live 2026-10-30 before this guard: all three shapes answered
    // 201 at signup and minted an account.
    for label in domain.split('.') {
        if label.starts_with('-') || label.ends_with('-') {
            return Err("email: a part of the domain starts or ends with a hyphen".into());
        }
    }
    if domain.contains('[') || domain.contains(']') {
        return Err("email: the domain must be a name, not a bracketed address".into());
    }
    if local.contains('"') {
        return Err("email: must not contain quote marks".into());
    }

    Ok(value)
}

/// The lookup key for an address a caller only needs to MATCH against a stored row
/// (`login`, `forgot-password`). Same trim+lowercase as [`normalize`], with no failure arm: a
/// malformed value simply matches nothing, so credential endpoints keep answering their own
/// "invalid email or password" / "if the email exists…" response instead of becoming an
/// account-existence oracle. Pair it with `WHERE lower(email) = $1` so rows stored before the
/// normalisation existed still resolve.
pub fn lookup_key(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Is this a usable address on a LEAD row? The same syntax rules as [`normalize`], answering
/// `Ok(())` or a caller-safe reason so a capture handler can refuse a typo while the person is
/// still on the page.
///
/// An EMPTY value is allowed on purpose: a lead may legitimately arrive with only a name and a
/// phone, and not every capture form asks for an address. What is refused is a NON-empty value no
/// mail system could deliver — `leads.email` is what campaigns and the customer's own mail steps
/// later send TO, so a stored `not-an-email` is not a lead detail, it is a guaranteed bounce that
/// spends the sending domain's reputation. Measured live 2026-10-29 before this guard:
/// `POST /api/v1/leads` with `{"email":"not-an-email"}` answered **201** and wrote the row (same on
/// IncentiveSwift and ADASwift).
///
/// No normalisation: the value is stored exactly as typed, so this only decides acceptance.
pub fn check_lead_address(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Ok(());
    }
    normalize(raw).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_the_literal_that_minted_the_dead_account() {
        // The row that exposed this class in the fleet: users.email = 'bad'.
        assert_eq!(
            normalize("bad"),
            Err("email: must look like name@example.com".to_string())
        );
    }

    #[test]
    fn accepts_real_addresses_and_normalises_them() {
        assert_eq!(
            normalize("  Ada.Lovelace+trial@Example.COM  ").unwrap(),
            "ada.lovelace+trial@example.com"
        );
        assert_eq!(normalize("a@b.co").unwrap(), "a@b.co");
        // IDN domain, unicode local part, long-but-legal address.
        assert_eq!(normalize("User@München.DE").unwrap(), "user@münchen.de");
        assert!(normalize("öhn@example.com").is_ok());
        let long = format!("{}@example.com", "a".repeat(240));
        assert!(normalize(&long).is_ok());
    }

    #[test]
    fn plus_aliases_dots_and_subdomains_stay_valid() {
        for ok in [
            "a+b@x.com",
            "a.b.c@x.com",
            "user@mail.co.uk",
            "user@sub.domain.example.org",
            "user_1-2%3@x-y.com",
        ] {
            assert!(normalize(ok).is_ok(), "{ok} must stay valid");
        }
    }

    #[test]
    fn rejects_shapes_that_are_not_addresses() {
        for bad in [
            "",
            "   ",
            "bad",
            "@x.com",
            "user@",
            "user@nodot",
            "user@@x.com",
            "us er@x.com",
            "user@x .com",
            ".user@x.com",
            "user.@x.com",
            "us..er@x.com",
            "user@.x.com",
            "user@x..com",
            "user@x.com.",
            "user@x@y.com",
            // Measured live 2026-10-30: these three minted an account (201) before the guard.
            "user@-.com",
            "user@x-.com",
            "user@[1.2.3.4]",
            "\"ab\"@x.com",
        ] {
            assert!(normalize(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn normalisation_is_idempotent() {
        let once = normalize("  Zed+1@Example.COM ").unwrap();
        assert_eq!(normalize(&once).unwrap(), once);
    }

    #[test]
    fn lookup_key_matches_what_normalize_stores() {
        assert_eq!(lookup_key("  Zaarhub@gmail.com "), "zaarhub@gmail.com");
        // A malformed value has no failure arm here — it just matches nothing.
        assert_eq!(lookup_key("bad"), "bad");
    }

    #[test]
    fn a_lead_may_carry_no_address_but_not_an_impossible_one() {
        // A lead with only a name and a phone is a complete lead.
        assert!(check_lead_address("").is_ok());
        assert!(check_lead_address("   ").is_ok());
        assert!(check_lead_address("ada@example.com").is_ok());
        // Acceptance only — the stored value is never rewritten by this check.
        assert!(check_lead_address("Ada@Example.COM").is_ok());
        for bad in [
            "not-an-email",
            "no-at-sign",
            "@x.com",
            "user@nodot",
            "a b@x.com",
        ] {
            assert!(check_lead_address(bad).is_err(), "{bad:?} must be refused");
        }
    }
}
