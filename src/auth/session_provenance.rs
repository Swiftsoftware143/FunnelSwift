//! Was this session issued by this app, or minted by hand with the app's secret? (kanban t_a14ad547)
//!
//! WHY THIS EXISTS (measured 2026-11-26 23:19:12Z, production): a live probe read the real EMI Mold
//! Removal user's row and `JWT_SECRET` out of the app's own environment, minted an HS256 token for
//! that customer, and pressed Upgrade. The refusal arm filed a `plan_requests` row and mailed BOTH
//! the customer ("We have your FunnelSwift plan request — Agency / Scale") and the operator. Nothing
//! in the write path or the mail path could tell it was machinery, because the token was
//! cryptographically identical to the customer's own session.
//!
//! The `X-Swift-Harness` guard (`crate::probe_harness::write_verdict`, enforced at
//! `auth::global_auth::require_auth`) closes that for a run that DECLARES itself. This closes it for
//! one that does not: every token this app issues carries its own provenance (`Claims::origin`), and
//! a token minted outside the app cannot have one. The two doors that take money or mail a customer —
//! `POST /api/v1/plan-requests` and `POST /api/v1/checkout/create` — require it.
//!
//! NOT a cryptographic boundary, and the comment says so on purpose: whoever holds `JWT_SECRET` can
//! forge the claim as easily as the signature. It is a MACHINERY boundary. The fleet's probe
//! snippets copy a Claims literal out of this repo; a copied literal that predates `origin` (or an
//! agent that writes its own) now answers 403 with a sentence that names the rule, instead of
//! pretending to be a customer.
//!
//! GRANDFATHER CLAUSE: sessions issued before this release carry no `origin` and are still real
//! customers (a 30-day token does not grow a new field). `iat` before `PROVENANCE_EPOCH` is accepted;
//! a token minted afterwards must name where it came from. The epoch is the release time and can be
//! moved forward with `SESSION_PROVENANCE_EPOCH` once every live session has been replaced.

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use sqlx::PgPool;

/// Sessions minted before this instant (unix seconds) predate the `origin` claim and are accepted.
///
/// 2026-11-27T17:20:00Z — the release that introduced `Claims::origin`. Overridable so a later
/// rollout can advance it without a rebuild.
const DEFAULT_PROVENANCE_EPOCH: i64 = 1_795_802_400;

/// The sentence a refused caller reads. Plain, and it says what to do.
pub const SESSION_NOT_ISSUED: &str =
    "This session was not issued by signing in to FunnelSwift, so it cannot file a plan request or \
     start a checkout. Sign in normally and try again — nothing was recorded and no mail was sent.";

/// The epoch above, or the operator's override. A malformed override falls back to the constant
/// rather than to "accept everything".
pub fn provenance_epoch() -> i64 {
    std::env::var("SESSION_PROVENANCE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_PROVENANCE_EPOCH)
}

/// The pure decision, so it can be unit-tested without a database.
///
/// `via_api_key` — a durable key is an issued credential (tenant-scoped, minted in Settings), so it
/// is admitted: an operator's integration may file a request on a workspace's behalf.
/// `impersonating` — the console's own impersonation session is a real sign-in by the operator.
/// `tenant_is_probe` — a probe acting on its OWN probe-marked workspace is machinery doing machinery
/// work; it never touches a customer, and this is how a copy proves the flow.
/// `origin` — set by every mint site in this app; absent on a hand-minted token.
/// `iat` — before the epoch, a session predates the claim and is admitted.
pub fn may_use_money_door(
    via_api_key: bool,
    impersonating: bool,
    tenant_is_probe: bool,
    origin: Option<&str>,
    iat: i64,
) -> bool {
    if via_api_key || impersonating || tenant_is_probe {
        return true;
    }
    if origin.is_some_and(|o| !o.trim().is_empty()) {
        return true;
    }
    iat < provenance_epoch()
}

/// The gate both money/mail doors call, with the one database read it needs (is this workspace a
/// probe workspace?). A database error is a refusal, never a pass.
pub async fn ensure_issued_session(pool: &PgPool, auth: &AuthUser) -> Result<(), AppError> {
    let tenant_is_probe = match uuid::Uuid::parse_str(auth.tenant_id.trim()) {
        Ok(id) => match crate::probe_harness::tenant_is_probe(pool, id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("session provenance: tenant lookup failed: {e}");
                return Err(AppError::Internal(
                    "Could not verify this session's workspace. Nothing was recorded.".into(),
                ));
            }
        },
        Err(_) => false,
    };
    if may_use_money_door(
        auth.via_api_key,
        auth.impersonating.is_some(),
        tenant_is_probe,
        auth.origin.as_deref(),
        auth.iat,
    ) {
        return Ok(());
    }
    tracing::warn!(
        user = %auth.user_id,
        tenant = %auth.tenant_id,
        email = %auth.email,
        iat = auth.iat,
        "REFUSED a money/mail door for a session this app never issued (hand-minted token)"
    );
    Err(AppError::Forbidden(SESSION_NOT_ISSUED.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issued_session_passes() {
        let now = provenance_epoch() + 10;
        assert!(may_use_money_door(false, false, false, Some("login"), now));
        assert!(may_use_money_door(false, false, false, Some("signup"), now));
    }

    #[test]
    fn a_hand_minted_session_is_refused() {
        let now = provenance_epoch() + 10;
        assert!(!may_use_money_door(false, false, false, None, now));
        // an empty/blank origin is not a provenance
        assert!(!may_use_money_door(false, false, false, Some("   "), now));
    }

    #[test]
    fn the_three_issued_credentials_pass_without_the_claim() {
        let now = provenance_epoch() + 10;
        assert!(may_use_money_door(true, false, false, None, now)); // durable API key
        assert!(may_use_money_door(false, true, false, None, now)); // operator impersonation
        assert!(may_use_money_door(false, false, true, None, now)); // probe's own workspace
    }

    #[test]
    fn a_session_from_before_the_claim_is_grandfathered() {
        let old = provenance_epoch() - 1;
        assert!(may_use_money_door(false, false, false, None, old));
        // ...and one minted at exactly the epoch is NOT (the epoch is the first covered instant)
        assert!(!may_use_money_door(
            false,
            false,
            false,
            None,
            provenance_epoch()
        ));
    }
}
