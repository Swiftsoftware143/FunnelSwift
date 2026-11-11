//! `probe_addr` — recipient addresses a fleet harness owns, which must never be handed to a real
//! mail relay (kanban t_36b55ed2).
//!
//! WHY THIS EXISTS
//!
//! Every probe in this app's suite mints a throwaway account through the real signup route, and the
//! signup route mails the generated first password. The addresses those probes use are the fleet's
//! own dev domains — `swiftsoftware.dev`, `swiftsoftware.net` — which are REAL, routable domains:
//! measured on `mail.funnelswift.net` (Mailgun events), a message to `zzprobe-*@swiftsoftware.net`
//! reports `delivered`, i.e. it lands in the fleet's own mailbox. David therefore received mail from
//! machinery that was never meant to talk to him, and each probe also burned a delivery on the
//! domain's sending reputation.
//!
//! The policy (`/opt/swift/docs/fleet-probe-residue-policy-2026-09-28.md`) classes
//! `swiftsoftware.dev/.local` as FLEET-DEV — the harness class — and requires attribution to be
//! DATA. This module is the outbound half of that: a signup addressed into the harness class is
//! created normally but its credentials mail is never sent, so a probe can never reach an inbox.
//!
//! RESERVED CLASS — SUPPRESSED TOO (kanban t_e4d94bd9)
//!
//! The RFC 2606 / 6761 names (`example.com/.net/.org`, `*.invalid`, `*.test`) cannot resolve to a
//! mailbox, so a send to them can only bounce. Measured on the sibling CoreSwift-CRM domain
//! (t_e4d94bd9): harness rows addressed there produced 16 real bounces in ~5 minutes (`accepted`
//! then `failed`, Mailgun code 498) — and a sustained bounce rate is how Mailgun disables a domain,
//! which would take out the transactional mail of every app on the shared account. Those names are
//! therefore suppressed here too: the relay is never handed a message it can only bounce.
//!
//! ONE DELIBERATE EXCEPTION — `*.local` / `localhost`. Those stay OPEN because the content-level
//! harness `scripts/fs-cr1-credential-proof.py` points the provider at a local SMTP sink and signs
//! up as `cr1sink<hex>@probe.local` to read the credential message off the wire; suppressing `.local`
//! would delete the only harness that can prove the send itself still works. Only the fleet's own
//! `swiftsoftware.local` identity is listed, never the bare `local` TLD.
//!
//! The other half of the fix is the tenant marker (`crate::probe_harness`): a harness that sends
//! `X-Swift-Harness` marks its tenant, and the signup path suppresses on the marker too — so a probe
//! is silenced even when it addresses a routable domain this list does not know about.

/// The fleet's own harness domains. Every one of them is fleet-controlled: a message addressed here
/// can only land in the fleet's own mailboxes, never in a customer's.
pub const FLEET_HARNESS_DOMAINS: &[&str] = &[
    "swiftsoftware.dev",
    "swiftsoftware.net",
    "swiftsoftware.local",
    "example.invalid",
    "example.com",
    "example.net",
    "example.org",
    "invalid",
    "test",
];

/// Domains that make a SIGNUP fleet machinery WITHOUT being mail-suppressed.
///
/// `maxxspace.com` is the one domain served by the fleet's own disposable inbox (mail.tm): no
/// customer can own an address there, so a workspace minted on it is a probe by construction. Its
/// mail must still be DELIVERED, though — the credential-delivery proof reads the generated password
/// out of that inbox, so adding it to [`FLEET_HARNESS_DOMAINS`] would delete the only end-to-end
/// proof that a new customer's password reaches a mailbox. Hence a separate list, consulted by
/// [`attribution_domain`] and never by the mail layer.
///
/// Measured 2026-10-24 on FunnelSwift: `probe-attrpre1792882105@maxxspace.com` minted the tenant
/// "Sink Probe attrpre1792882105's Workspace" with `probe_harness` NULL, so it sat in the operator's
/// tenant list as a real customer until it was deleted by hand. The five sibling apps were fixed the
/// same day; this is the last door.
pub const ATTRIBUTION_ONLY_DOMAINS: &[&str] = &["maxxspace.com"];

/// The domain that makes a SIGNUP a probe: the fleet-harness class of [`harness_domain`] plus the
/// attribution-only class above. The mail layer must keep using [`harness_domain`], never this.
pub fn attribution_domain(addr: &str) -> Option<&'static str> {
    if let Some(domain) = harness_domain(addr) {
        return Some(domain);
    }
    let (_, domain) = addr.rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    ATTRIBUTION_ONLY_DOMAINS.iter().copied().find(|d| {
        domain == *d || (domain.len() > d.len() + 1 && domain.ends_with(&format!(".{d}")))
    })
}

/// The fleet harness domain `addr` belongs to, if any. A subdomain counts
/// (`probe@mail.swiftsoftware.dev` is still the harness class); a domain that merely CONTAINS the
/// name does not (`x@notswiftsoftware.dev` is refused — the match is on the label boundary).
///
/// Everything is normalised first (trim, lowercase, trailing dot), so `Probe@SwiftSoftware.NET.`
/// classifies like the exact form. A value with no `@`, or with an empty domain, is not an address
/// and returns `None` — the caller decides what to do with an unparseable address, this function
/// only answers the harness question.
pub fn harness_domain(addr: &str) -> Option<&'static str> {
    let (_, domain) = addr.rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    FLEET_HARNESS_DOMAINS.iter().copied().find(|d| {
        domain == *d || (domain.len() > d.len() + 1 && domain.ends_with(&format!(".{d}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_harness_domains_are_detected() {
        // The addresses the suite actually uses (measured off mail.funnelswift.net).
        assert_eq!(
            harness_domain("zzprobe-profile-1791480727@swiftsoftware.net"),
            Some("swiftsoftware.net")
        );
        assert_eq!(
            harness_domain("smoke-kinetic-1234@swiftsoftware.dev"),
            Some("swiftsoftware.dev")
        );
        assert_eq!(
            harness_domain("x@swiftsoftware.local"),
            Some("swiftsoftware.local")
        );
        // Case, whitespace and a trailing root dot are normalised away.
        assert_eq!(
            harness_domain(" Probe@SwiftSoftware.NET. "),
            Some("swiftsoftware.net")
        );
        // A subdomain is still the harness class.
        assert_eq!(
            harness_domain("probe@mail.swiftsoftware.dev"),
            Some("swiftsoftware.dev")
        );
        // Reserved RFC 2606 / 6761 names are suppressed too (kanban t_e4d94bd9): a send to them can
        // only bounce, and bounces are how a sending domain gets disabled.
        assert_eq!(harness_domain("x@example.com"), Some("example.com"));
        assert_eq!(harness_domain("probe@example.org"), Some("example.org"));
        assert_eq!(harness_domain("probe@example.net"), Some("example.net"));
        assert_eq!(harness_domain("x@foo.invalid"), Some("invalid"));
        assert_eq!(harness_domain("x@foo.test"), Some("test"));
    }

    #[test]
    fn real_customer_addresses_are_not_suppressed() {
        // A customer's mailbox, including the fleet's own REAL brand domain (.com is not a harness
        // domain — the policy classes it human-possible) and the owner's proof inbox.
        assert_eq!(harness_domain("certifiedtb143@yahoo.com"), None);
        assert_eq!(harness_domain("someone@gmail.com"), None);
        assert_eq!(harness_domain("david@swiftsoftware.com"), None);
        assert_eq!(harness_domain("support@funnelswift.net"), None);
        // A domain that merely CONTAINS a harness domain is not a match.
        assert_eq!(harness_domain("x@notswiftsoftware.dev"), None);
        assert_eq!(harness_domain("x@swiftsoftware.dev.evil.com"), None);
        // `*.local` stays OPEN on purpose (the local SMTP-sink credential harness needs it — see the
        // module doc). Everything else reserved is suppressed now.
        assert_eq!(harness_domain("cr1sink0a1b@probe.local"), None);
        assert_eq!(harness_domain("x@probe.local"), None);
    }

    #[test]
    fn malformed_values_never_classify() {
        assert_eq!(harness_domain("no-at-sign"), None);
        assert_eq!(harness_domain(""), None);
        // Domain-only classification: an empty local part is still the harness domain. Such an
        // address never gets this far (the signup normalises and rejects it first), and suppressing
        // is the safe direction if one ever did.
        assert_eq!(
            harness_domain("@swiftsoftware.dev"),
            Some("swiftsoftware.dev")
        );
        assert_eq!(harness_domain("trailing@"), None);
    }
}

/// The funnel's disposable inbox is a SIGNUP tell, never a mail-suppression tell.
#[cfg(test)]
mod attribution_only_tests {
    use super::*;

    #[test]
    fn the_disposable_inbox_attributes_a_signup() {
        assert_eq!(
            attribution_domain("probe-swsink165312@maxxspace.com"),
            Some("maxxspace.com")
        );
        assert_eq!(
            attribution_domain("Probe@MAIL.MaxxSpace.COM."),
            Some("maxxspace.com")
        );
        // A domain that merely CONTAINS the name is somebody else's mailbox.
        assert_eq!(attribution_domain("person@notmaxxspace.com"), None);
        assert_eq!(attribution_domain("person@maxxspace.com.evil.net"), None);
    }

    #[test]
    fn the_mail_layer_never_suppresses_the_disposable_inbox() {
        // The credential-delivery proof reads its password back out of that inbox: if the MAIL
        // class ever grew to include maxxspace.com the proof would silently stop receiving mail.
        assert_eq!(harness_domain("probe@maxxspace.com"), None);
        assert_eq!(
            attribution_domain("probe@example.invalid"),
            Some("example.invalid")
        );
        assert_eq!(
            harness_domain("probe@example.invalid"),
            Some("example.invalid")
        );
        assert_eq!(attribution_domain("customer@gmail.com"), None);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SYNTHETIC REQUESTERS — a harness must never page a human (kanban t_f17b5802)
//
// Measured 2026-11-10: one day of audit walks produced 49 operator alert emails
// ("A support request needs an answer: …", "A customer asked to buy …") because the
// plan-request and support-request doors notify the OPERATOR by design — the recipient is
// the operator whatever address the requester gave. The probe rows were all minted on
// reserved addresses (`@example.invalid`, `p3-smoke@x.com`), and the app dutifully paged a
// person for every one of them.
//
// The row is the record: it still lands in the console queue, the request still works, and
// the caller still answers 200/201. Only the ALERT to a human is withheld — the same
// treatment the signup and password-reset paths already give mail that can only bounce.
//
// The predicate is deliberately on the REQUESTER, never on the route: the same door serves
// real customers, and a real customer's request must still reach a human.
// ─────────────────────────────────────────────────────────────────────────────

/// Domains that can only belong to a harness. `x.com` / `test.com` are added on top of the
/// RFC-reserved class because the fleet's own proof scripts address them and they are not
/// mail-suppressed (they are real, registrable domains), so nothing else would stop the alert.
pub const SYNTHETIC_REQUESTER_DOMAINS: &[&str] = &[
    "example.invalid",
    "example.test",
    "example.com",
    "x.com",
    "test.com",
];

/// Local parts a harness mints. `bounce+` is the shape a deliverability probe uses.
pub const SYNTHETIC_LOCAL_PREFIXES: &[&str] = &["probe-", "canary-", "bounce+", "verify"];

/// Workspace/account name fragments that name fleet machinery in words.
pub const SYNTHETIC_NAME_FRAGMENTS: &[&str] =
    &["probe", "canary", "synthetic", "smoke test", "test farm"];

/// Why `email`/`name` is a synthetic (harness) identity, or `None` for a real customer.
///
/// Returns the offending domain, local-part prefix or name fragment, so the caller can log a
/// reason instead of a bare boolean. Matches on the label boundary (`x@notexample.com` is a
/// customer), normalises case/whitespace/trailing root dot, and never treats an empty string
/// as a match.
pub fn synthetic_requester(email: &str, name: &str) -> Option<&'static str> {
    let addr = email.trim();
    // The fleet's own harness domains (swiftsoftware.dev/.net/.local) are already classified.
    if let Some(domain) = harness_domain(addr) {
        return Some(domain);
    }
    if let Some((local, domain)) = addr.rsplit_once('@') {
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        if !domain.is_empty() {
            if let Some(d) = SYNTHETIC_REQUESTER_DOMAINS.iter().copied().find(|d| {
                domain == *d || (domain.len() > d.len() + 1 && domain.ends_with(&format!(".{d}")))
            }) {
                return Some(d);
            }
        }
        let local = local.trim().to_ascii_lowercase();
        if let Some(p) = SYNTHETIC_LOCAL_PREFIXES
            .iter()
            .copied()
            .find(|p| local.starts_with(p))
        {
            return Some(p);
        }
    }
    let name = name.trim().to_ascii_lowercase();
    if !name.is_empty() {
        if let Some(f) = SYNTHETIC_NAME_FRAGMENTS
            .iter()
            .copied()
            .find(|f| name.contains(f))
        {
            return Some(f);
        }
    }
    None
}

#[cfg(test)]
mod synthetic_requester_tests {
    use super::*;

    #[test]
    fn harness_identities_are_synthetic() {
        // The exact shapes that produced the 49 alerts.
        assert_eq!(
            synthetic_requester("probe-ctrl-1111@example.invalid", ""),
            Some("example.invalid")
        );
        assert_eq!(synthetic_requester("p3-smoke@x.com", ""), Some("x.com"));
        assert_eq!(synthetic_requester("walk@test.com", ""), Some("test.com"));
        // `test` is the wider harness label, so it wins the match; either way it is refused.
        assert!(synthetic_requester("someone@mail.example.test", "").is_some());
        assert_eq!(
            synthetic_requester("probe-abc@real-customer.com", ""),
            Some("probe-")
        );
        assert_eq!(synthetic_requester("canary-7@acme.io", ""), Some("canary-"));
        assert_eq!(synthetic_requester("bounce+x@acme.io", ""), Some("bounce+"));
        assert_eq!(synthetic_requester("verify1@acme.io", ""), Some("verify"));
        // The name alone is enough: a harness workspace on a plausible address.
        assert_eq!(synthetic_requester("a@b.com", "Probe Walk"), Some("probe"));
        assert_eq!(
            synthetic_requester("a@b.com", "Smoke Test 4"),
            Some("smoke test")
        );
        assert_eq!(
            synthetic_requester("a@b.com", "Synthetic Co"),
            Some("synthetic")
        );
        assert_eq!(
            synthetic_requester("a@b.com", "Test Farm 2"),
            Some("test farm")
        );
    }

    #[test]
    fn real_customers_are_not_synthetic() {
        assert_eq!(
            synthetic_requester("owner@albert-tire.com", "Albert Tire"),
            None
        );
        assert_eq!(synthetic_requester("certifiedtb143@yahoo.com", ""), None);
        assert_eq!(
            synthetic_requester("dev@notexample.com", "Not Example Ltd"),
            None
        );
        assert_eq!(synthetic_requester("x@notx.com", ""), None);
        assert_eq!(synthetic_requester("", ""), None);
        assert_eq!(synthetic_requester("no-at-sign", "Real Co"), None);
    }
}
