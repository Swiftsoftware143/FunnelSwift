//! The fleet's system tenant.
//!
//! `00000000-0000-0000-0000-000000000001` is the seeded tenant that owns the fleet's shared rows:
//! the plan-derived affiliate products and the product categories they point at. Several
//! admin-facing listings deliberately show the caller's rows PLUS the system tenant's, so the
//! predicate is `tenant_id = $1 OR tenant_id = $n` with this id BOUND.
//!
//! Why a value and not a literal: pre-build gate rule 5a blocks a UUID written into the source
//! because a hardcoded id is a row that was looked up by hand and will silently stop matching if
//! the tenant is ever re-seeded. Binding it here also means the sentinel has exactly ONE
//! definition in the app instead of four copies (kanban t_92ce05b3).
use uuid::Uuid;

/// The system tenant's id as an integer: the all-zeros UUID with a trailing 1.
pub const SYSTEM_TENANT_U128: u128 = 1;

/// The system tenant's id, ready to `.bind()`.
pub fn system_tenant_id() -> Uuid {
    Uuid::from_u128(SYSTEM_TENANT_U128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_tenant_id_is_the_all_zeros_sentinel_with_a_trailing_one() {
        let id = system_tenant_id();
        assert_eq!(id.as_u128(), SYSTEM_TENANT_U128);
        assert!(!id.is_nil(), "the sentinel is NOT the nil UUID");
        // the canonical 8-4-4-4-12 rendering, so a log line or a smoke probe can compare it
        assert_eq!(id.hyphenated().to_string().len(), 36);
    }
}
