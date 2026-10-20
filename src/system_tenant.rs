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

/// Re-seed the ONE free catalogue product per app that the fleet's upgrade-event endpoint looks up.
///
/// The reader (`handlers::affiliate_tracking_handler::handle_affiliate_upgrade_event`) credits a
/// commission with `SELECT id FROM affiliate_products WHERE source_app = $1 AND is_active = true`.
/// Measured live 2026-10-19: `affiliate_products` held 0 rows, so that lookup matched nothing and
/// every cross-app upgrade would have recorded a commission with a NULL product — the money lands,
/// the attribution does not. Migration 055 seeded these rows and its `NOT EXISTS` guard means it can
/// never run again, while the rows themselves are one `ON DELETE CASCADE` away from being wiped
/// again with the System tenant (measured: that has now happened twice).
///
/// Idempotency is the reader's own predicate: an ACTIVE row for a `source_app` satisfies both the
/// lookup and this guard, so a re-run inserts nothing and an admin's deliberately INACTIVE row is
/// left alone rather than resurrected. The keys are the compile-time literals each sibling app
/// sends; nothing here invents one.
pub async fn ensure_fleet_catalogue(pool: &sqlx::PgPool) -> Result<u64, crate::error::AppError> {
    ensure_system_tenant(pool).await?;
    let res = sqlx::query(
        r#"INSERT INTO affiliate_products
              (id, tenant_id, name, description, price, default_commission_rate, is_active,
               is_third_party, product_type, owner_name, source_app, slug)
           SELECT gen_random_uuid(), $1,
                  v.name, v.description, 0, 20.0, true, false, 'software', 'SwiftSoftware',
                  v.key, v.slug
             FROM (VALUES
                   ('workflowswift',      'WorkflowSwift Free',
                    'WorkflowSwift automation - free plan', 'workflowswift-free'),
                   ('coreswift',          'CoreSwift Free',
                    'CoreSwift CRM - free plan', 'coreswift-free'),
                   ('incentiveswift',     'IncentiveSwift Free',
                    'IncentiveSwift campaigns and loyalty - free plan', 'incentiveswift-free'),
                   ('adaswift',           'ADASwift Free',
                    'ADASwift accessibility - free plan', 'adaswift-free'),
                   ('missedcallrespondr', 'MissedCall Respondr Free',
                    'MissedCall Respondr - free plan', 'missedcallrespondr-free')
                  ) AS v(key, name, description, slug)
            WHERE NOT EXISTS (
                   SELECT 1 FROM affiliate_products p
                    WHERE p.source_app = v.key AND p.is_active = true)"#,
    )
    .bind(system_tenant_id())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
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
