-- Migration 087: "one tag routes one ACTIVE product" must also hold on the is_active transition.
--
-- WHY THIS FILE EXISTS (kanban t_5196edda, measured 2026-10-02 while proving t_c149b025)
--
-- 075 declared the rule, and its trigger body carried this early return:
--
--     IF TG_OP = 'UPDATE' AND NEW.system_tag_id IS NOT DISTINCT FROM OLD.system_tag_id THEN RETURN NEW
--
-- Re-activating a RETIRED product does not change its tag, so the guard short-circuited and the tag
-- ended up routing TWO active products. Measured inside a rolled-back transaction on the live
-- database (audits/t_c149b025/26-guards-hole-output.txt) and again through the SERVED admin
-- console's own retirement checkbox (audits/t_c149b025 leg L6): create X with tag T, retire X,
-- create Y with T, re-activate X -> 200, and
--
--     select count(*) from affiliate_products where system_tag_id = T and is_active   -- = 2
--
-- The tag reader (tag_logic::attribute_affiliate_on_tags) resolves the product for a lead's tag and
-- with two active rows carrying it the attribution - and therefore the commission credited - is
-- decided by the reader's ORDER BY rather than by the catalogue. This is the shape t_68e92c8e closed
-- for source_app keys (075's own unique index), one column over.
--
-- THE ARM (the card's arm (a): the arm with a database-level guarantee, 075's own pattern)
--
--   * uq_affiliate_products_active_tag - a PARTIAL UNIQUE INDEX on system_tag_id WHERE is_active,
--     the index half of 075's own uq_affiliate_products_active_key shape. Its predicate is exactly
--     the trigger's "this row routes a tag" condition, so it cannot refuse a retired row, a row with
--     no tag, or two rows that are not both active.
--   * the trigger is REPLACED, not dropped: its fired-condition widens to include is_active (so a
--     writer that only flips the flag is still checked) and its early return narrows to "the row was
--     ALREADY active and the tag is unchanged" - a genuine no-op for this rule. Everything else (a
--     new row, a tag change, an ACTIVATION) is checked and refused with the conflict still NAMED, as
--     075 promised. The index is the TOCTOU backstop behind it, exactly as it is for the key rule.
--   * the guarded de-dupe of 075 step 3 is re-asserted, with 075's OWN total winner rule, so this
--     file also holds on a database restored from a dump older than 075 (the index cannot be created
--     over a tag that already routes two active rows).
--
-- The app half (src/handlers/affiliate_product_handler.rs) mirrors the widened fired-condition in
-- `ensure_tag_route_free` so the panel is refused BEFORE the write with a 409 naming the holder, and
-- `map_product_write_error` recognises BOTH database refusals (the trigger's P0001 and the new
-- index's 23505) and answers the SAME 409 - so no operator ever sees the anonymous 500 that
-- t_c149b025 removed.
--
-- NO SEMICOLONS IN THIS HEADER (the deploy staging path splits on the statement separator, as 050,
-- 0059, 060, 074 and 075 record).

DO $mig$
DECLARE
    tag_demoted int := 0;
    idx_created boolean := false;
    fn_replaced boolean := false;
    trg_replaced boolean := false;
    dup_tags    text;
    holders     text;
BEGIN
    IF to_regclass('public.affiliate_products') IS NULL THEN
        RAISE NOTICE '087: skipped - affiliate_products absent on this database';
        RETURN;
    END IF;

    -- 1. Guard: a system TAG routing more than one ACTIVE product (075 step 3, re-asserted so this
    --    file holds on a database restored from a dump older than 075). Winner rule is 075's own,
    --    verbatim, so a re-run picks the same survivor: the row a commission points at, then the row
    --    with a caller key, then the oldest created_at, then the lowest id.
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

    -- 2. THE ENFORCEMENT, database-level: at most one ACTIVE product per system tag. Partial ON
    --    PURPOSE - the predicate IS the trigger's own "this row routes a tag" condition, so a retired
    --    row, a row with no tag, and (as before) the whole plan-derived catalogue stay legal.
    IF NOT EXISTS (SELECT 1 FROM pg_indexes
                    WHERE schemaname = 'public'
                      AND indexname = 'uq_affiliate_products_active_tag') THEN
        CREATE UNIQUE INDEX uq_affiliate_products_active_tag
            ON affiliate_products (system_tag_id)
         WHERE is_active AND system_tag_id IS NOT NULL;
        idx_created := true;
    END IF;

    -- 3. THE ENFORCEMENT, named conflict: 075's function, with its fired-condition widened.
    IF to_regclass('public.tags') IS NOT NULL THEN
        CREATE OR REPLACE FUNCTION enforce_one_active_product_per_system_tag()
        RETURNS TRIGGER AS $fn$
        DECLARE
            clash text;
        BEGIN
            -- Not routing this tag at all (no tag, or the row is not active): nothing to enforce.
            IF NEW.system_tag_id IS NULL OR NEW.is_active IS NOT TRUE THEN
                RETURN NEW;
            END IF;
            -- The row was ALREADY routing this tag and still is: the is_active transition cannot have
            -- changed anything for this rule. NOTE the narrowness - the 075 form of this early return
            -- did not look at is_active, which is the hole this migration closes.
            IF TG_OP = 'UPDATE'
               AND NEW.system_tag_id IS NOT DISTINCT FROM OLD.system_tag_id
               AND OLD.is_active IS TRUE THEN
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
        fn_replaced := true;

        -- is_active joins the fired-columns list: a writer that only flips the flag must still be
        -- checked (the app does, but a rule that only lives in one write path is a rule that will be
        -- bypassed - 070's own reason for the trigger).
        DROP TRIGGER IF EXISTS trg_one_active_product_per_tag ON affiliate_products;
        CREATE TRIGGER trg_one_active_product_per_tag
            BEFORE INSERT OR UPDATE OF system_tag_id, is_active
            ON affiliate_products
            FOR EACH ROW
            EXECUTE FUNCTION enforce_one_active_product_per_system_tag();
        trg_replaced := true;
    END IF;

    -- 4. Post-state as NOTICEs only (075's own reason: this app's runner logs a migration error and
    --    starts anyway, so a hard assert would refuse the boot and print nothing useful).
    SELECT string_agg(t.name || ' x' || n, ', ') INTO dup_tags
      FROM (SELECT t.name, count(*) AS n
              FROM affiliate_products p JOIN tags t ON t.id = p.system_tag_id
             WHERE p.is_active
             GROUP BY t.name HAVING count(*) > 1) t;
    SELECT string_agg(t.name || ' -> ' || p.name, ', ' ORDER BY t.name) INTO holders
      FROM affiliate_products p JOIN tags t ON t.id = p.system_tag_id
     WHERE p.is_active;

    RAISE NOTICE '087: demoted % duplicate tag row(s); unique index created=%, function replaced=%, trigger replaced=%',
                 tag_demoted, idx_created, fn_replaced, trg_replaced;
    RAISE NOTICE '087: system tags routing more than one active product AFTER this migration: %',
                 coalesce(dup_tags, '(none)');
    RAISE NOTICE '087: the tags routing an active product now: %', coalesce(holders, '(none)');
END $mig$;
