-- Migration 092: a commission GROUP belongs to a tenant (kanban t_e8a297fb).
--
-- WHY THIS FILE EXISTS
--
-- 065 created `affiliate_product_groups.tenant_id` with the rest of the commission model, and NOTHING
-- ever wrote it: `create_group` inserted `(id, name, commission_rate, description)` only, so every group
-- row was ownerless (`tenant_id IS NULL`), no read or write carried a tenant predicate, and the whole
-- feature was FLEET-GLOBAL for an `is_admin` caller. MEASURED 2026-10-02 (binary 99c2a99b, ledger 89,
-- /opt/swift/audits/t_db5d07aa/CENSUS.md, 0 groups live): an admin of one workspace could
--
--   * GET   /api/v1/affiliate-product-groups          -> every group on the fleet,
--   * PUT   /api/v1/affiliate-product-groups/:id      -> re-rate another workspace's group,
--   * DELETE/api/v1/affiliate-product-groups/:id      -> delete another workspace's group,
--   * POST  /api/v1/affiliate-product-groups[/:id/products] -> put ANOTHER workspace's product into a
--     group, whose rate then decided what that product PAID (`src/commission.rs` rule 2 outranks rules
--     3-6, the same money path t_db5d07aa closed for plan-derived products).
--
-- THE DECISION (arm (a) of the card): the feature is TENANT-SCOPED, and the scope is the contract
-- `POST /api/v1/affiliate-product-rates/bulk` was given in t_5c2a9bde:
--
--     tenant_id = the CALLER's tenant  OR  tenant_id = the fleet's SYSTEM tenant
--
-- The SYSTEM arm is not a hole: the SYSTEM tenant owns the fleet-shared rows the admin product list
-- already renders (the plan-derived catalogue and the sibling apps' free products), so an admin who may
-- put one of those in a group may edit the group that speaks for it. A group is created in the CALLER's
-- tenant, never in SYSTEM, so the SYSTEM arm only ever addresses rows a migration or repair placed there.
-- Arm (b) — keeping the feature deliberately fleet-global and documenting it — was NOT taken: it is a
-- cross-tenant write on a money path, and the sibling rate route had already been scoped, so one of the
-- two surfaces would have had to disagree with the other.
--
-- WHAT THIS FILE CHANGES STRUCTURALLY (the app predicates are the fix; this is the backstop)
--
--   An ownerless group can no longer exist. There is deliberately NO DEFAULT on the column: a writer
--   that forgets the tenant must FAIL LOUDLY (23502, a not-null violation the app's error mapper turns
--   into a 500 the log names) rather than silently land in the fleet-shared bucket — a default of the
--   SYSTEM sentinel would have been exactly the silent cross-tenant write this card is about. Measured
--   live before this file was written: ownerless groups 0, so this is a no-op on the fleet's data and a
--   guard on its future.
--
-- Additive and reversible: one backfill (of nothing, live) plus a NOT NULL on a column that already
-- holds no NULL. No column is dropped, renamed or narrowed; no row is deleted or re-rated.

-- ── 1. attribute every existing ownerless group to the fleet's SYSTEM tenant ─────────────────────
-- The sentinel is DERIVED (the tenant that owns the plan-derived catalogue), not written here: a
-- hardcoded uuid is a row looked up by hand that silently stops matching if the tenant is re-seeded
-- (pre-build gate rule 5a), and `src/system_tenant.rs` is the app's one definition of it.
DO $$
DECLARE
    sys_tenant uuid;
    n_ownerless bigint;
    n_moved bigint := 0;
BEGIN
    SELECT count(*) INTO n_ownerless FROM affiliate_product_groups WHERE tenant_id IS NULL;
    SELECT ap.tenant_id INTO sys_tenant
      FROM affiliate_products ap
     WHERE ap.plan_id IS NOT NULL
     ORDER BY ap.tenant_id
     LIMIT 1;

    IF n_ownerless > 0 THEN
        IF sys_tenant IS NULL THEN
            -- Cannot be attributed to anything, and inventing an owner would move rows between
            -- workspaces. Fail the boot instead: a fleet with ownerless groups needs a human decision.
            RAISE EXCEPTION
                '092: % commission group(s) have no tenant and no SYSTEM tenant could be derived from '
                'the plan-derived catalogue — attribute them by hand before this migration can run',
                n_ownerless;
        END IF;
        UPDATE affiliate_product_groups SET tenant_id = sys_tenant WHERE tenant_id IS NULL;
        GET DIAGNOSTICS n_moved = ROW_COUNT;
    END IF;

    RAISE NOTICE
        '092: affiliate_product_groups.tenant_id: % ownerless group(s); % attributed to the SYSTEM tenant %; the column becomes NOT NULL (kanban t_e8a297fb)',
        n_ownerless, n_moved, coalesce(sys_tenant::text, '(none derivable)');
END $$;

-- ── 2. structural: a group can no longer exist without an owner ──────────────────────────────────
-- Idempotent: re-running SET NOT NULL on a column that is already NOT NULL is a no-op.
ALTER TABLE affiliate_product_groups ALTER COLUMN tenant_id SET NOT NULL;

COMMENT ON COLUMN affiliate_product_groups.tenant_id IS
    'The workspace whose admin created the group (NOT NULL since 092, kanban t_e8a297fb). Every read and write of a group is scoped to `tenant_id = caller OR tenant_id = SYSTEM` — the same two-tenant contract POST /api/v1/affiliate-product-rates/bulk uses (t_5c2a9bde). No default on purpose: a writer that forgets the tenant must fail loudly, never land in the fleet-shared bucket.';
