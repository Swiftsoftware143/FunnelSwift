-- Migration 075: make "one ACTIVE product per caller key" an INVARIANT, not an accident.
--
-- WHY THIS FILE EXISTS (kanban t_68e92c8e, measured 2026-10-02)
--
-- Two readers attribute a cross-app conversion to a product with
--
--     SELECT id FROM affiliate_products WHERE source_app = $1 AND is_active = true LIMIT 1
--
-- (handlers/affiliate_tracking_handler.rs, handlers/cross_app_webhook_handler.rs — both now call the
-- single resolver src/affiliate_products.rs). LIMIT 1 with NO ORDER BY is planner-dependent, and
-- NOTHING in the schema kept a key to one row: the invariant held only because of the migrations that
-- happened to create the rows.
--
--   * 043 seeded one product per app under the System tenant and spelled MissedCall's key
--     'missedcall', which no caller ever sends (055's own header).
--   * that tenant was CASCADE-deleted, 055 re-inserted the five keys the fleet actually sends
--     (workflowswift, coreswift, incentiveswift, adaswift, missedcallrespondr — each a compile-time
--     literal in the sending app) and backfilled source_app 'funnelswift' onto every plan-derived row.
--   * 074 retires 043's stale 'missedcall' row on a from-zero build.
--
-- MEASURED BEFORE THIS FILE, on BOTH the live database and an EMPTY database booted with the shipped
-- binary: 7 active products each, and the census is identical key for key — every caller key resolves
-- exactly ONE active product, and no system tag routes more than one. So the duplicate data the card
-- measured has already been retired by 074. What is NOT gone is the ability to re-create it: an admin
-- product, a plan, or a future migration can add a second active row and the reader silently picks
-- either one, and 060's own boot NOTICE ("tags routing more than one active product") is the only
-- thing that would say so. This file removes that class.
--
-- THE DECISION (the card's arm (a) ENFORCE — the stronger fix; the reader's plan-aware ORDER BY in
-- src/affiliate_products.rs is its completion for the one genuinely multi-row key)
--
--   * ONE ACTIVE PRODUCT PER source_app KEY, enforced by a PARTIAL UNIQUE INDEX. Partial ON PURPOSE
--     so it is plan-aware: a platform-wide free product (one per caller key, plan_id NULL) is unique,
--     while FunnelSwift's per-free-plan rows (plan_id set, minted by 067 as 'FunnelSwift ' ||
--     plan.name) are keyed by their plan and may share the 'funnelswift' key. A blanket NOT NULL plus
--     unique index was refused, measured: affiliate_product_handler::create_affiliate_product inserts
--     a product with NO source_app at all, and a NULL source_app can never satisfy source_app = $1, so
--     it is not a key — and NOT NULL would refuse the admin product form.
--   * ONE PRODUCT PER SYSTEM TAG, enforced by a TRIGGER that names the conflict, because system_tag_id
--     IS writable from the product form (create and update) so a bare unique index would surface as an
--     anonymous 500 there. This is 070's own pattern and 070's own reason: a routing rule that lives in
--     one write path is a rule that will be bypassed. 060 declares the rule itself — it prints
--     'tags routing more than one active product (the reader takes them all)' as a defect on every boot.
--
-- WINNER RULE for the de-dupe guards below. They find NO duplicates on any current database (measured
-- on live and from-zero: 0 demoted), so they are the guard for a database restored from an older dump
-- or copied from one. When they do fire, the survivor is the row that owns a system tag (the tag
-- reader's routing signal), then the row an existing commission already points at (never orphan a live
-- attribution), then the OLDEST created_at, then the lowest id — a total order, so a re-run picks the
-- same winner, and the boot log names every survivor.
--
-- NO SEMICOLONS IN THIS HEADER (the deploy staging path splits on the statement separator, as 050,
-- 0059, 060 and 074 record).

DO $mig$
DECLARE
    legacy_retired int := 0;
    demoted        int := 0;
    tag_demoted    int := 0;
    idx_created    boolean := false;
    dup_keys       text;
    dup_tags       text;
    survivors      text;
BEGIN
    IF to_regclass('public.affiliate_products') IS NULL THEN
        RAISE NOTICE '075: skipped - affiliate_products absent on this database';
        RETURN;
    END IF;

    -- 1. Retire 043's never-sent legacy key wherever it still exists. 074 does the same on a from-zero
    --    build — re-asserted here so the invariant follows from THIS file too, whichever route created
    --    the row. Guarded exactly like 074 part 2: only the never-sent key, only when the corrected row
    --    is present, and never when a commission points at it (the FK is ON DELETE SET NULL, so this
    --    guard is what stops a real attribution being nulled).
    DELETE FROM affiliate_products ap
     WHERE ap.source_app = 'missedcall'
       AND EXISTS (SELECT 1 FROM affiliate_products p2
                    WHERE p2.source_app = 'missedcallrespondr' AND p2.is_active)
       AND NOT EXISTS (SELECT 1 FROM affiliate_commissions c WHERE c.product_id = ap.id);
    GET DIAGNOSTICS legacy_retired = ROW_COUNT;

    -- 2. Guard: a caller key with more than one ACTIVE, non-plan row. Retire all but the winner.
    WITH ranked AS (
        SELECT p.id,
               row_number() OVER (
                   PARTITION BY p.source_app
                   ORDER BY (p.system_tag_id IS NOT NULL) DESC,
                            (EXISTS (SELECT 1 FROM affiliate_commissions c
                                      WHERE c.product_id = p.id)) DESC,
                            p.created_at ASC,
                            p.id ASC
               ) AS rn
          FROM affiliate_products p
         WHERE p.is_active
           AND p.source_app IS NOT NULL
           AND p.plan_id IS NULL
    )
    UPDATE affiliate_products p
       SET is_active = false, updated_at = NOW()
      FROM ranked r
     WHERE r.id = p.id AND r.rn > 1;
    GET DIAGNOSTICS demoted = ROW_COUNT;

    -- 3. Guard: a system TAG routing more than one ACTIVE product. The tag reader
    --    (tag_logic::attribute_affiliate_on_tags) takes them ALL and writes one commission each, so
    --    more than one is a double attribution. Retire all but the winner.
    WITH ranked AS (
        SELECT p.id,
               row_number() OVER (
                   PARTITION BY p.system_tag_id
                   ORDER BY (EXISTS (SELECT 1 FROM affiliate_commissions c
                                      WHERE c.product_id = p.id)) DESC,
                            (p.source_app IS NOT NULL) DESC,
                            p.created_at ASC,
                            p.id ASC
               ) AS rn
          FROM affiliate_products p
         WHERE p.is_active AND p.system_tag_id IS NOT NULL
    )
    UPDATE affiliate_products p
       SET is_active = false, updated_at = NOW()
      FROM ranked r
     WHERE r.id = p.id AND r.rn > 1;
    GET DIAGNOSTICS tag_demoted = ROW_COUNT;

    -- 4. THE ENFORCEMENT: at most one ACTIVE product per source_app key. Partial, so plan-derived rows
    --    (plan_id set) are excluded and FunnelSwift's one-row-per-free-plan catalogue stays legal.
    IF NOT EXISTS (SELECT 1 FROM pg_indexes
                    WHERE schemaname = 'public'
                      AND indexname = 'uq_affiliate_products_active_key') THEN
        CREATE UNIQUE INDEX uq_affiliate_products_active_key
            ON affiliate_products (source_app)
         WHERE is_active AND source_app IS NOT NULL AND plan_id IS NULL;
        idx_created := true;
    END IF;

    -- 5. THE ENFORCEMENT, tag side: one tag routes one product, refused with the conflict named.
    IF to_regclass('public.tags') IS NOT NULL THEN
        CREATE OR REPLACE FUNCTION enforce_one_active_product_per_system_tag()
        RETURNS TRIGGER AS $fn$
        DECLARE
            clash text;
        BEGIN
            IF NEW.system_tag_id IS NULL OR NEW.is_active IS NOT TRUE THEN
                RETURN NEW;
            END IF;
            IF TG_OP = 'UPDATE' AND NEW.system_tag_id IS NOT DISTINCT FROM OLD.system_tag_id THEN
                RETURN NEW;
            END IF;
            SELECT p.name INTO clash
              FROM affiliate_products p
             WHERE p.system_tag_id = NEW.system_tag_id
               AND p.is_active
               AND p.id <> NEW.id
             LIMIT 1;
            IF clash IS NOT NULL THEN
                RAISE EXCEPTION
                    'affiliate product "%" cannot be routed by that system tag: product "%" already is. '
                    'One tag routes one product — retire that product first, or use another tag.',
                    COALESCE(NEW.name, NEW.id::text), clash;
            END IF;
            RETURN NEW;
        END;
        $fn$ LANGUAGE plpgsql;

        DROP TRIGGER IF EXISTS trg_one_active_product_per_tag ON affiliate_products;
        CREATE TRIGGER trg_one_active_product_per_tag
            BEFORE INSERT OR UPDATE OF system_tag_id
            ON affiliate_products
            FOR EACH ROW
            EXECUTE FUNCTION enforce_one_active_product_per_system_tag();
    END IF;

    -- 6. Post-state as NOTICEs only. A hard assert would refuse the boot on an empty database and this
    --    app's runner logs a migration error and starts anyway (src/db.rs), which is exactly how a
    --    silent no-op would hide. The index and the trigger are the assertions that carry.
    SELECT string_agg(k || ' x' || n, ', ') INTO dup_keys
      FROM (SELECT source_app AS k, count(*) AS n
              FROM affiliate_products
             WHERE is_active AND source_app IS NOT NULL AND plan_id IS NULL
             GROUP BY source_app HAVING count(*) > 1) d;
    SELECT string_agg(t || ' x' || n, ', ') INTO dup_tags
      FROM (SELECT t.name AS t, count(*) AS n
              FROM affiliate_products p JOIN tags t ON t.id = p.system_tag_id
             WHERE p.is_active
             GROUP BY t.name HAVING count(*) > 1) d;
    SELECT string_agg(source_app || ' -> ' || name, ', ' ORDER BY source_app) INTO survivors
      FROM affiliate_products
     WHERE is_active AND source_app IS NOT NULL AND plan_id IS NULL;

    RAISE NOTICE '075: retired % stale legacy row(s), demoted % duplicate key row(s), % duplicate tag row(s); unique index created=%',
                 legacy_retired, demoted, tag_demoted, idx_created;
    RAISE NOTICE '075: caller keys with more than one active product AFTER this migration: %',
                 coalesce(dup_keys, '(none)');
    RAISE NOTICE '075: system tags routing more than one active product AFTER this migration: %',
                 coalesce(dup_tags, '(none)');
    RAISE NOTICE '075: the one active product per caller key now: %', coalesce(survivors, '(none)');
END $mig$;
