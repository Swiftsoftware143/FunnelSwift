-- Migration 083: a plan-derived affiliate product's rate is NOT writable - the plan owns it
-- (kanban t_5c2a9bde, which ENFORCES the decision t_92bd5eb6 recorded and migration 076 normalised).
--
-- WHY THIS FILE EXISTS
--
-- 076 recorded and repaired the decision "for a plan-derived affiliate product (`plan_id IS NOT
-- NULL`), `plans.commission_rate` IS the product's `default_commission_rate`", but nothing stopped a
-- write path from moving the value again. Measured on 2026-10-02, the ONE resolver
-- (`src/commission.rs`) ranks `affiliate_products.default_commission_rate` ABOVE `plans.commission_rate`,
-- and the authoritative conversion receiver (`src/handlers/cross_app_webhook_handler.rs`, the "THE
-- rate" arm) calls it - so whatever sits in that column is what a sale PAYS. Two writers could still
-- move it on a plan-derived row:
--
--   * `POST /api/v1/affiliate-product-rates/bulk` - an `UPDATE ... WHERE id = ANY($1)` with NO tenant
--     filter and NO plan-derived guard, wired to the www-admin console's "Set a rate for selected
--     products" tick-box form whose list INCLUDES the System-tenant plan-derived rows.
--   * `PUT /api/v1/affiliate-products/:id` - writes the caller's `default_commission_rate` for any row
--     its `id = $1 AND tenant_id = $2` predicate reaches.
--
-- The drift was also invisible and self-erasing: `plan_handler::sync_plan_to_affiliate_product`
-- rewrites the column from the plan on the next plan save, so the paid rate changed and then silently
-- changed back, with no record of which value was in force when a conversion landed. This is a money
-- field, so the rule now lives in the DATABASE as well as in both handlers.
--
-- THE DECISION (the card's arm (a) - make the recorded decision structural, NOT reverse it)
--
--   A plan-derived product has no rate of its own. Any write that would leave
--   `default_commission_rate` different from the rate on the plan it points at is REFUSED with a
--   message that names the plan to edit. The legitimate writers are unaffected, measured:
--     * `plan_handler::sync_plan_to_affiliate_product` writes the plan's own `commission_rate` off the
--       `plans` row, so its UPDATE and its INSERT both satisfy the invariant by construction.
--     * migration 076 (and the repair at the top of this file) write the plan's rate to the rows that
--       disagree, so they are the repair path rather than a violation.
--     * a product with `plan_id IS NULL` (a sibling app's free product, or an admin product) is not
--       plan-derived and this file has no opinion about it.
--   Arm (b) - declaring the product's own rate authoritative for plan-derived rows too - was NOT
--   taken: it is the opposite of t_92bd5eb6, it would need `sync_plan_to_affiliate_product` to stop
--   mirroring the plan, and the value it would bless (10.00) is measurably the column DEFAULT written
--   by a literal 10.0 in materialisation routes that t_6d326447 retired.
--
-- HOW THE REFUSAL READS TO AN OPERATOR
--
--   The trigger raises with a user-defined SQLSTATE `SW001` (class `SW` cannot collide with any
--   PostgreSQL or SQL-standard code) and a message that names the product and the plan. Without this
--   file the write path's own check is the only thing that could explain the refusal; WITH it, a
--   writer that bypasses the handlers (psql, a restored dump being re-saved, a future route) still
--   cannot drift the column - and the handlers map the raise to the SAME readable 409 instead of
--   `error.rs`'s anonymous `500 "Database error"` (the class t_c149b025 fixed for the 075 tag rule).
--
-- WHAT THIS FILE DELIBERATELY DOES NOT DO
--
--   * It does not touch `affiliate_commissions`, `affiliate_conversions` or any settled amount - the
--     history of what was PAID is not the rate, and a rate rule must never rewrite a payout.
--   * It does not constrain the OTHER rate columns. `affiliate_products.commission_rate` is the
--     legacy column the resolver reads only when `default_commission_rate` is NULL, and
--     `affiliate_product_groups.commission_rate` is a deliberate operator override that OUTRANKS the
--     product's own rate by design (`src/commission.rs`, rule 2). Changing either is a different
--     decision than this card asked for.
--   * It does not make the rate column read-only for every row - only a plan-derived row is a mirror.
--   * It does not refuse a write that keeps the value EQUAL to the plan's rate, so re-saving a product
--     (rename, description, tag) keeps working.
--
-- NO SEMICOLONS IN THIS HEADER (the deploy staging path splits on the statement separator, as 050,
-- 0059, 060, 074, 075 and 076 record).

DO $mig$
DECLARE
    repaired      int := 0;
    stale_before  text;
    stale_after   text;
    catalogue     text;
    trigger_now   boolean := false;
BEGIN
    IF to_regclass('public.affiliate_products') IS NULL
       OR to_regclass('public.plans') IS NULL THEN
        RAISE NOTICE '083: skipped - affiliate_products or plans absent on this database';
        RETURN;
    END IF;

    -- 1. Repair first, so the trigger below can never be installed onto a database that already
    --    violates the rule (an old dump restored onto a newer binary, or a hand edit). This is 076's
    --    own normalisation re-asserted - idempotent, NULL-safe, and a measured no-op on live.
    SELECT coalesce(string_agg(ap.name || ' (' || ap.default_commission_rate || ' -> '
                               || p.commission_rate || ')', ', ' ORDER BY ap.name), '(none)')
      INTO stale_before
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id
     WHERE ap.default_commission_rate IS DISTINCT FROM p.commission_rate;

    UPDATE affiliate_products ap
       SET default_commission_rate = p.commission_rate,
           updated_at = NOW()
      FROM plans p
     WHERE ap.plan_id = p.id
       AND ap.default_commission_rate IS DISTINCT FROM p.commission_rate;
    GET DIAGNOSTICS repaired = ROW_COUNT;

    -- 2. THE GUARD. A plan-derived row's rate must equal its plan's rate, whatever the writer.
    --    `UPDATE OF default_commission_rate, plan_id` keeps every unrelated UPDATE (is_active, a
    --    rename that does not carry the column, the plan-delete deactivation) out of this function.
    CREATE OR REPLACE FUNCTION enforce_plan_derived_product_rate()
    RETURNS TRIGGER AS $fn$
    DECLARE
        plan_rate numeric(5,2);
        plan_name text;
    BEGIN
        IF NEW.plan_id IS NULL THEN
            RETURN NEW;
        END IF;
        SELECT p.commission_rate, p.name INTO plan_rate, plan_name
          FROM plans p WHERE p.id = NEW.plan_id;
        -- No plan row (unreachable behind the plan_id FK, which is ON DELETE SET NULL) or a plan with
        -- no rate: the database has no opinion about a value it cannot derive.
        IF plan_rate IS NULL THEN
            RETURN NEW;
        END IF;
        IF NEW.default_commission_rate IS NOT DISTINCT FROM plan_rate THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION USING
            ERRCODE = 'SW001',
            MESSAGE = format(
                'the commission rate on affiliate product "%s" comes from its plan "%s" (%s%%) - '
                'this product mirrors its plan and has no rate of its own, so edit the plan to change '
                'the rate',
                coalesce(NEW.name, NEW.id::text), coalesce(plan_name, 'unknown'), plan_rate),
            HINT = 'DECISION t_92bd5eb6, enforced by migration 083 (kanban t_5c2a9bde).';
    END;
    $fn$ LANGUAGE plpgsql;

    DROP TRIGGER IF EXISTS trg_plan_derived_product_rate ON affiliate_products;
    CREATE TRIGGER trg_plan_derived_product_rate
        BEFORE INSERT OR UPDATE OF default_commission_rate, plan_id
        ON affiliate_products
        FOR EACH ROW
        EXECUTE FUNCTION enforce_plan_derived_product_rate();
    trigger_now := true;

    -- 3. Post-state as NOTICEs only. A hard assert would refuse the boot on an empty database, and
    --    this app's runner logs a migration error and starts anyway (src/db.rs) - which is exactly how
    --    a silent no-op hides. The trigger is the assertion that carries.
    SELECT coalesce(string_agg(ap.name || ' = ' || ap.default_commission_rate
                               || ' (plan ' || p.commission_rate || ')', ', ' ORDER BY ap.name),
                    '(none)')
      INTO stale_after
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id
     WHERE ap.default_commission_rate IS DISTINCT FROM p.commission_rate;

    SELECT coalesce(string_agg(ap.name || ' = ' || ap.default_commission_rate, ', ' ORDER BY ap.name),
                    '(none)')
      INTO catalogue
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id;

    RAISE NOTICE '083: re-rated % plan-derived product(s) to their plan rate; rows named before: %',
                 repaired, stale_before;
    RAISE NOTICE '083: plan-derived rows still differing from their plan after this file: %',
                 stale_after;
    RAISE NOTICE '083: the plan-derived catalogue now: %', catalogue;
    RAISE NOTICE '083: the rate on a plan-derived product is now refused at the write path (SQLSTATE '
                 'SW001), trigger installed: %, writable-it-is-not', trigger_now;
END $mig$;
