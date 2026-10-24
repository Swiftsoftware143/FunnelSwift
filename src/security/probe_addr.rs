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
