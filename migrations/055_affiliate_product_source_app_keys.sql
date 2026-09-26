-- Migration 055: give affiliate_products.source_app the values the fleet actually sends.
--
-- The reader (src/handlers/affiliate_tracking_handler.rs:206, handle_affiliate_upgrade_event, plus
-- the same predicate in cross_app_webhook_handler.rs:181) credits a product with
--
--     SELECT id FROM affiliate_products WHERE source_app = $1 AND is_active = true LIMIT 1
--
-- bound to the payload's source_app. Measured live on 2026-09-25: all 6 rows carried an empty
-- source_app, so the lookup could never match and affiliate_commissions.product_id was NULL for
-- every commission the endpoint writes. The money landed, the attribution did not.
--
-- Why the rows were missing rather than merely unset: 043 seeded one product per app under the
-- System tenant (00000000-0000-0000-0000-000000000001). That tenant no longer exists on this
-- database, and both tags.tenant_id and affiliate_products.tenant_id are FOREIGN KEY ... ON DELETE
-- CASCADE, so deleting it took 043's 7 system tags and 12 products with it. 043 is recorded in
-- _sqlx_migrations as applied and its recorded checksum matches the file byte for byte, so it will
-- never re-run - hence this migration.
--
-- The keys below are not invented. Each app sends a compile-time literal:
--   WorkflowSwift/src/handlers/plan_handler.rs       "source_app": "workflowswift"
--   ADASwift/src/handlers/plans_handler.rs           "source_app": "adaswift"
--   IncentiveSwift/src/handlers/plans_handler.rs     "source_app": "incentiveswift"
--   CoreSwift-CRM/src/billing/handlers.rs            "source_app": "coreswift"
--   missedcallrespondr/src/handlers/plans_handler.rs "source_app": "missedcallrespondr"
-- 043 spelled the last one 'missedcall', which no caller ever sends - corrected here.

DO $mig$
DECLARE
    keys_present text;
    dups text;
BEGIN
    IF to_regclass('public.affiliate_products') IS NULL
       OR to_regclass('public.tenants') IS NULL THEN
        RAISE NOTICE '055: skipped - affiliate_products or tenants absent on this database';
        RETURN;
    END IF;

    -- 1. The System tenant is the declared home of platform-wide catalog rows: 000001_initial.sql:250
    --    seeds it, affiliate_product_handler.rs:125 lists products owned by the caller's tenant OR
    --    by that id, and 043 put these very products under it. Restore it if it is gone.
    INSERT INTO tenants (id, name, slug)
    VALUES ('00000000-0000-0000-0000-000000000001', 'System', 'system')
    ON CONFLICT DO NOTHING;

    -- 2. One ACTIVE product per source_app the endpoint is called with. is_active = true is part of
    --    the reader's own predicate, so that is the idempotency key: a re-run inserts nothing, and
    --    an admin's INACTIVE row neither satisfies the reader nor this guard.
    INSERT INTO affiliate_products
        (id, tenant_id, name, description, price, default_commission_rate, is_active,
         is_third_party, product_type, owner_name, source_app, slug)
    SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001',
           v.name, v.description, 0, 20.0, true, false, 'software', 'SwiftSoftware', v.key, v.slug
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
             WHERE p.source_app = v.key AND p.is_active = true);

    -- 3. The plan-derived products (plan_id set) are written by plan_handler::sync_plan_to_affiliate,
    --    whose INSERT passes source_app 'funnelswift'. Rows that predate migration 026's column are
    --    still NULL because the sync's UPDATE arm never revisits source_app - backfill them to the
    --    value the app's own writer uses. Their own routing key is plan_id / system_tag_id (the
    --    fleet's upgrade-event endpoint is called by the OTHER apps only), so more than one row may
    --    share the 'funnelswift' key - reported below.
    UPDATE affiliate_products
       SET source_app = 'funnelswift', updated_at = NOW()
     WHERE source_app IS NULL AND plan_id IS NOT NULL;

    -- 4. Post-state as NOTICEs only. A hard assert would refuse the boot on an empty database, and
    --    this app's runner logs a migration error and starts anyway (src/db.rs:36), which is exactly
    --    how a silent no-op would hide.
    SELECT coalesce(string_agg(DISTINCT source_app, ', ' ORDER BY source_app), '(none)')
      INTO keys_present
      FROM affiliate_products
     WHERE source_app IS NOT NULL AND is_active = true;
    RAISE NOTICE '055: active source_app keys now: %', keys_present;

    SELECT string_agg(k || ' x' || n, ', ') INTO dups
      FROM (SELECT source_app AS k, count(*) AS n
              FROM affiliate_products
             WHERE source_app IS NOT NULL AND is_active = true
             GROUP BY source_app HAVING count(*) > 1) d;
    RAISE NOTICE '055: keys with more than one active product (the reader uses LIMIT 1): %',
                 coalesce(dups, '(none)');
END $mig$;
