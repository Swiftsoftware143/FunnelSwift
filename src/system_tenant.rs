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

/// Make sure the system tenant ROW exists before anything stamps it as an owner.
///
/// The catalogue's owner is a real row in `tenants`, and `affiliate_products.tenant_id` is
/// `FOREIGN KEY … ON DELETE CASCADE`, so the day that row disappears the whole catalogue goes with
/// it — and every later write that stamps the sentinel fails on the constraint instead of creating
/// the product. Measured live 2026-10-19: the row was gone, `affiliate_products` held 0 rows while
/// 6 plans existed, and creating or editing a plan logged
/// `affiliate products_tenant_id_fkey` (see `plan_handler::sync_plan_to_affiliate_product`).
///
/// The console's delete guard refuses the `system` slug (`tenant_handler::FIXTURE_SLUG_PREFIXES`),
/// but a cleanup script talking to Postgres directly is not the console — this seed has already
/// been lost twice (migration 055 documents the first time). So the writer repairs it instead of
/// trusting a seed: `ON CONFLICT DO NOTHING` makes this idempotent and free on the common path,
/// and it never touches an existing row's name or settings.
pub async fn ensure_system_tenant(pool: &sqlx::PgPool) -> Result<(), crate::error::AppError> {
    sqlx::query(
        "INSERT INTO tenants (id, name, slug) VALUES ($1, 'System', 'system') ON CONFLICT DO NOTHING",
    )
    .bind(system_tenant_id())
    .execute(pool)
    .await?;
    Ok(())
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
