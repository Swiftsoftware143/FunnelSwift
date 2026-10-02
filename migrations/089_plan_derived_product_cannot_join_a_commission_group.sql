-- Migration 089: a PLAN-DERIVED affiliate product may not be a member of a commission group
-- (kanban t_db5d07aa — the same money risk as t_5c2a9bde, through a DIFFERENT column).
--
-- WHY THIS FILE EXISTS
--
-- 083 made a plan-derived product's OWN rate column unwritable, because `src/commission.rs` ranks that
-- column ABOVE `plans.commission_rate`. But the resolver's order is
--
--     1. affiliates.override_commission_rate
--     2. affiliate_product_groups.commission_rate   <-- a GROUP rate
--     3. affiliate_products.default_commission_rate (locked to the plan by 083)
--     4. affiliate_products.commission_rate (legacy)
--     5. affiliates.commission_rate
--     6. plans.commission_rate
--     7. the 20% fallback
--
-- so a GROUP rate (rule 2) still outranked the plan's rate for a plan-derived product. MEASURED LIVE on
-- 2026-10-02 (binary 12afd2da, /opt/swift/audits/t_db5d07aa/grouprate-proof-pre.txt, 24 assertions): a
-- plan-derived product at a 7.50 plan rate, put in a group whose rate is 35, resolved to
-- `source=product_group rate=35.0` — the money path paid 35, not 20/7.5, and 083's trigger never fired
-- because the write was to `affiliate_products.group_id`, not to the rate column. Three doors reached it:
--   * POST /api/v1/affiliate-product-groups with `product_ids` naming the product (201, assigned),
--   * POST /api/v1/affiliate-product-groups/:id/products (replace the membership),
--   * PUT  /api/v1/affiliate-product-groups/:id setting the rate on a group that already had it.
--
-- THE DECISION (the card's arm (a) — the plan owns the rate, so a group may not cover one of its mirrors)
--
--   A plan-derived product (`plan_id IS NOT NULL`) is a MIRROR of its plan and has no rate of its own,
--   so it may not be placed in a commission group at all. Allowed, deliberately:
--     * a product with `plan_id IS NULL` (a sibling app's free product, an admin product) groups and
--       re-rates exactly as before — the operator override rule 2 exists for is untouched, and the
--       console keeps offering it.
--     * a group with NO rate is a pure label and remains legal for any product; it changes no money.
--     * the OTHER direction of this same state (setting a group's rate while a plan-derived product is
--       already a member) cannot leave a violation either: no plan-derived member can exist, so there is
--       nothing for the group's rate to speak for.
--   Arm (b) — declaring a group rate authoritative over the plan for a plan-derived product — was NOT
--   taken: it would reverse t_92bd5eb6 through a second column, it explains nothing on the screen where
--   the plan's rate is edited, and `sync_plan_to_affiliate_product` would keep rewriting the mirror
--   underneath it, leaving two values that disagree with no record of which one was in force.
--
-- HOW THE REFUSAL READS TO AN OPERATOR
--
--   SQLSTATE `SW002` (class `SW`, user-defined, so it cannot collide with any PostgreSQL or SQL-standard
--   code — `SW001` is 083's own) with a message that names the product and the plan. The two writers of
--   `affiliate_products.group_id` refuse the request BEFORE writing and answer the same readable 409
--   (`affiliate_commission_handler::plan_derived_group_conflict`), and map this raise to that same 409
--   rather than `error.rs`'s anonymous `500 "Database error"`. WITH this file a writer that bypasses the
--   handlers (psql, a restored dump being re-saved, a future route) still cannot build the state.
--
-- WHAT THIS FILE DELIBERATELY DOES NOT DO
--
--   * It does not touch `affiliate_commissions` or any settled amount — the history of what was PAID is
--     not the rate.
--   * It does not constrain `affiliate_product_groups.commission_rate` itself, nor `group_id` on a row
--     whose `plan_id` is NULL, nor the `ON DELETE SET NULL` that a group deletion performs.
--   * It does not scope the group routes to a tenant. Those routes are `is_admin`-only and the group
--     table carries no tenant of its own; that is a separate question, recorded in this card's census,
--     not settled here.
--
-- NO SEMICOLONS IN THIS HEADER (the deploy staging path splits on the statement separator, as 050, 0059,
-- 060, 074, 075, 076 and 083 record).

DO $mig$
DECLARE
    repaired      int := 0;
    detached      text;
    still_grouped text;
    joined_now    text;
    trigger_now   boolean := false;
BEGIN
    IF to_regclass('public.affiliate_products') IS NULL
       OR to_regclass('public.affiliate_product_groups') IS NULL THEN
        RAISE NOTICE '089: skipped - affiliate_products or affiliate_product_groups absent on this database';
        RETURN;
    END IF;

    -- 1. Repair first, so the trigger below can never be installed onto a database that already
    --    violates the rule (an old dump restored onto a newer binary, or a hand edit). Detaching is the
    --    -only- repair that touches nothing but the illegal row: the product falls back to its plan's
    --    rate, which is the rate the rule says it pays, and every honest member of the group keeps its
    --    own. (Deleting the group would re-rate products this rule has no opinion about.) A no-op on
    --    live: 0 groups and 0 grouped products, measured 2026-10-02.
    SELECT coalesce(string_agg(ap.name || ' in group ' || g.name, ', ' ORDER BY ap.name), '(none)')
      INTO detached
      FROM affiliate_products ap
      JOIN affiliate_product_groups g ON g.id = ap.group_id
     WHERE ap.plan_id IS NOT NULL;

    UPDATE affiliate_products ap
       SET group_id = NULL,
           updated_at = NOW()
     WHERE ap.plan_id IS NOT NULL
       AND ap.group_id IS NOT NULL;
    GET DIAGNOSTICS repaired = ROW_COUNT;

    -- 2. THE GUARD. A plan-derived row may not name a group, whatever the writer.
    --    `UPDATE OF group_id, plan_id` keeps every unrelated UPDATE (a rename, is_active, a rate write —
    --    083's own territory) out of this function.
    CREATE OR REPLACE FUNCTION enforce_plan_derived_product_no_group()
    RETURNS TRIGGER AS $fn$
    DECLARE
        plan_name text;
        plan_rate numeric(5,2);
    BEGIN
        IF NEW.plan_id IS NULL OR NEW.group_id IS NULL THEN
            RETURN NEW;
        END IF;
        SELECT p.name, p.commission_rate INTO plan_name, plan_rate
          FROM plans p WHERE p.id = NEW.plan_id;
        RAISE EXCEPTION USING
            ERRCODE = 'SW002',
            MESSAGE = format(
                'affiliate product "%s" belongs to plan "%s" (%s%%) and may not be placed in a '
                'commission group - a plan-derived product mirrors its plan and has no rate of its own, '
                'so a group rate would outrank the plan. Edit the plan to change the rate',
                coalesce(NEW.name, NEW.id::text), coalesce(plan_name, 'unknown'), coalesce(plan_rate, 0)),
            HINT = 'DECISION kanban t_db5d07aa, enforced by migration 089.';
    END;
    $fn$ LANGUAGE plpgsql;

    DROP TRIGGER IF EXISTS trg_plan_derived_product_no_group ON affiliate_products;
    CREATE TRIGGER trg_plan_derived_product_no_group
        BEFORE INSERT OR UPDATE OF group_id, plan_id
        ON affiliate_products
        FOR EACH ROW
        EXECUTE FUNCTION enforce_plan_derived_product_no_group();
    trigger_now := true;

    -- 3. Post-state as NOTICEs only (a hard assert would refuse the boot on an empty database, and this
    --    app's runner logs a migration error and starts anyway — which is exactly how a silent no-op
    --    hides). The trigger is the assertion that carries.
    SELECT coalesce(string_agg(ap.name || ' (' || p.name || ' at ' || p.commission_rate || '%, '
                               || coalesce(g.name, 'ungrouped') || ')', ', ' ORDER BY ap.name), '(none)')
      INTO joined_now
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id
      LEFT JOIN affiliate_product_groups g ON g.id = ap.group_id;

    SELECT coalesce(string_agg(ap.name || ' group_id=' || ap.group_id, ', ' ORDER BY ap.name), '(none)')
      INTO still_grouped
      FROM affiliate_products ap
     WHERE ap.plan_id IS NOT NULL AND ap.group_id IS NOT NULL;

    RAISE NOTICE '089: detached % plan-derived product(s) from a group; they were: %',
                 repaired, detached;
    RAISE NOTICE '089: plan-derived products still naming a group after this file: %', still_grouped;
    RAISE NOTICE '089: the plan-derived catalogue now: %', joined_now;
    RAISE NOTICE '089: a group may no longer cover one (SQLSTATE SW002), trigger installed: %, '
                 'writable-it-is-not', trigger_now;
END $mig$;
