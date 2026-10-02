-- Migration 076: a plan-derived affiliate product's rate IS its plan's rate (kanban t_92bd5eb6).
--
-- THE DECISION (ONE), recorded here because this file is the guard that carries it.
--
--   For an affiliate product that exists only because a PLAN exists (affiliate_products.plan_id IS
--   NOT NULL), the plan's commission_rate IS the product's default_commission_rate. The product is
--   the plan's mirror and carries no commercial value of its own.
--
-- WHY THE PLAN'S RATE AND NOT THE PRODUCT'S OWN, measured rather than argued:
--
--   1. The money path reads the PRODUCT's rate, not the plan's. src/commission.rs (2026-09-29) is
--      the ONE place a rate is decided, and its order is: this affiliate's override, then the
--      product group, then affiliate_products.default_commission_rate, then the product's legacy
--      column, then affiliates.commission_rate, then plans.commission_rate, then the 20 percent
--      fallback. The product's own rate OUTRANKS the plan's, and the authoritative conversion
--      receiver (src/handlers/cross_app_webhook_handler.rs, the "THE rate" comment from 2026-09-29)
--      calls that resolver. So whatever sits in default_commission_rate is what a sale actually pays.
--   2. The rows that exist today were built that way already. Migration 067 inserted FunnelSwift's
--      two free products with BOTH rate columns set to the plan's own commission_rate, with the
--      comment "commission_rate comes from the plan itself, not from a number invented here".
--   3. The ONE writer agrees. plan_handler::sync_plan_to_affiliate_product (kanban t_6d326447)
--      takes no commercial value from any caller: it reads name, price and commission_rate off the
--      plans row. Callers only decide WHEN a product must exist.
--   4. The wrong value was never a decision. The card measured the six plan-derived rows carrying
--      default_commission_rate = 10.00 with an empty description and a NULL category_id, which is
--      exactly the column DEFAULT, written by a literal 10.0 in two materialisation routes that
--      t_6d326447 retired. "10.00 is deliberate" is measurably false, which is why the arm that
--      declares the product's rate authoritative was rejected.
--   5. A plan-derived product can only ever point at a FREE plan. Migration 070 refuses (by trigger)
--      an affiliate product linked to a plan whose price is not zero, so the catalogue is
--      FunnelSwift's two free entry points plus the sibling apps' free products.
--
-- WHAT THIS FILE TOUCHES: exactly one column, on plan-derived rows only.
--   UPDATE affiliate_products SET default_commission_rate = plans.commission_rate, updated_at = NOW()
--   It is guarded, NULL-safe and idempotent, and it is a measured no-op on the live database
--   (2026-10-02: 0 rows; the two live plan-derived rows already carry their plan's 20.00).
--
-- WHAT IT DELIBERATELY DOES NOT TOUCH. Every one of these was measured, not assumed:
--   * id, tenant_id, plan_id, group_id, system_tag_id, slug, source_app, product_type, owner_name,
--     is_active, is_third_party, url, price, name - none of them is a rate and none is in the SET
--     list. `is_active` in particular is owned by the product screen and by the plan-delete path
--     (kanban t_9c30ce49), and a rate repair must never resurrect a retired product.
--   * a row whose plan_id is NULL (a sibling app's free product) - it is not plan-derived, so this
--     file has no opinion about it and the WHERE never reaches it.
--   * description - NOT normalised. Migration 067 wrote deliberate operator-facing copy
--     ("FunnelSwift Capture Free - the capture-free entry point.") and the ONE writer rewrites it to
--     "<plan name> - FunnelSwift Plan" on the next plan save. Overwriting live copy is a UI change,
--     not the rate decision this card asked for.
--   * category_id - NOT normalised, same reason: it is a taxonomy/UI classification (currently NULL
--     on both live rows), the ONE writer already resolves it to the funnelswift-plans category, and
--     re-classifying live rows is not a money decision.
--   * any row whose two values already agree, so updated_at is not churned and a replay is a
--     genuine no-op (IS DISTINCT FROM, which is NULL-safe).
--   * affiliate_commissions, affiliate_conversions, affiliate_payouts, affiliate_links - the
--     conversion history records what was PAID, and a rate repair must never rewrite a settled
--     amount. None of those tables is named in this file.
--   * `plans.commission_rate IS NOT NULL` is part of the WHERE rather than a COALESCE, so a plan
--     with no rate could never blank a product's rate. The column is NOT NULL today (measured in
--     information_schema), so that arm is belt-and-braces rather than reachable.
--
-- WHY IT EXISTS AS A MIGRATION even though live needs no repair: the card's six-row shape (a
-- plan-derived product sitting at the column DEFAULT) can only come back two ways now - an old dump
-- restored onto a newer binary, or a hand edit - and this file is the guard for the first. The dry
-- run in /opt/swift/audits/t_92bd5eb6/ proves it moves exactly the wrong rows and nothing else.
--
-- NO SEMICOLONS IN THIS HEADER (the deploy staging path splits on the statement separator, as 050,
-- 0059, 060, 074 and 075 record).

DO $mig$
DECLARE
    moved        int := 0;
    stale_before text;
    stale_after  text;
    census_after text;
BEGIN
    IF to_regclass('public.affiliate_products') IS NULL
       OR to_regclass('public.plans') IS NULL THEN
        RAISE NOTICE '076: skipped - affiliate_products or plans absent on this database';
        RETURN;
    END IF;

    -- The rows this file exists to correct, named BEFORE the update so the boot log says which
    -- values moved (a silent UPDATE is how a money change goes unnoticed).
    SELECT coalesce(string_agg(ap.name || ' (' || ap.default_commission_rate || ' -> '
                               || p.commission_rate || ')', ', ' ORDER BY ap.name), '(none)')
      INTO stale_before
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id
     WHERE p.commission_rate IS NOT NULL
       AND ap.default_commission_rate IS DISTINCT FROM p.commission_rate;

    -- THE REPAIR. Only the rate column and updated_at, only on plan-derived rows.
    UPDATE affiliate_products ap
       SET default_commission_rate = p.commission_rate,
           updated_at = NOW()
      FROM plans p
     WHERE ap.plan_id = p.id
       AND p.commission_rate IS NOT NULL
       AND ap.default_commission_rate IS DISTINCT FROM p.commission_rate;
    GET DIAGNOSTICS moved = ROW_COUNT;

    SELECT coalesce(string_agg(ap.name || ' = ' || ap.default_commission_rate
                               || ' (plan ' || p.commission_rate || ')', ', ' ORDER BY ap.name),
                    '(none)')
      INTO stale_after
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id
     WHERE ap.default_commission_rate IS DISTINCT FROM p.commission_rate;

    SELECT coalesce(string_agg(ap.name || ' = ' || ap.default_commission_rate, ', ' ORDER BY ap.name),
                    '(none)')
      INTO census_after
      FROM affiliate_products ap
      JOIN plans p ON p.id = ap.plan_id;

    RAISE NOTICE '076: re-rated % plan-derived product(s) to their plan rate; rows named before: %',
                 moved, stale_before;
    RAISE NOTICE '076: plan-derived rows still differing from their plan after this file: %',
                 stale_after;
    RAISE NOTICE '076: the plan-derived catalogue now: %', census_after;
END $mig$;
