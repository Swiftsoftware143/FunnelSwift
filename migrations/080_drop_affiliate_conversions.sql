-- 080: DROP `affiliate_conversions` — the affiliate conversion ledger with a reader and no writer
-- (kanban t_4c634069; the residue one table over from `affiliate_clicks`, dropped by 073).
--
-- MEASURED on the live `funnelswift` database before this migration (2026-10-02 06:13 EDT):
--   * `affiliate_conversions` = **0 rows**. It has never held one in production.
--   * No writer anywhere in the product: `grep -rn "INSERT INTO affiliate_conversions"` across every
--     app's `src/`, `/opt/swift/scripts` and `/opt/swift/bin` returns exactly ONE hit — the seed line
--     of the smoke harness `/opt/swift/bin/fsw-affiliate-decode-smoke.py` (removed in the same pass,
--     see below). No Rust file in any app inserts a row.
--   * The only reader was `list_conversions` -> `GET /api/v1/affiliate-conversions`, deleted in the
--     same pass. A fleet-wide caller census (every app's `src/`, every served `www*` root, nginx,
--     n8n, scripts) found ZERO callers: no console, no portal, no other app, no workflow reads it.
--   * The POST on that same path is a different thing entirely and STAYS: `track_conversion` writes
--     the money ledger `affiliate_commissions` (status 'pending'), exactly like the two real credit
--     paths (`tag_logic.rs` attribution-at-signup and the cross-app upgrade webhook).
--   * `click_id` was the last FK into the dropped `affiliate_clicks` (declared by 062). 073 dropped
--     that constraint by name and left the column nullable, permanently NULL — its only reader was
--     the `list_conversions` SELECT. With this table gone the column goes with it; there is no
--     remaining reader of `click_id` anywhere.
--
-- VERDICT (same discipline as t_6a7e77e1 / 073): DROP, do not wire. Wiring would mean inventing a
-- second money ledger beside `affiliate_commissions` (different shape: affiliate_user_id/customer_id/
-- click_id vs affiliate_id/lead_id/product_id) for a flow the product already runs correctly
-- elsewhere. An empty table with no writer is a false oracle: every from-zero or schema audit reports
-- it as "a feature with data waiting", and `ARCHITECTURE.md` documented a write flow (steps 3 and 4
-- of the cross-app block) that had never run once. That doc block is rewritten onto
-- `affiliate_commissions` in the same pass.
--
-- 021 (`CREATE TABLE affiliate_conversions`) and 062 are NOT edited — an applied migration is
-- history. A rollback is the `CREATE TABLE` copied from 021 plus the FK from 062; the table is
-- empty, so nothing is lost.

DO $$
DECLARE n bigint;
BEGIN
    IF to_regclass('public.affiliate_conversions') IS NULL THEN
        RAISE NOTICE '080: affiliate_conversions already absent, nothing to drop';
        RETURN;
    END IF;
    SELECT count(*) INTO n FROM affiliate_conversions;
    IF n <> 0 THEN
        RAISE EXCEPTION '080: affiliate_conversions holds % rows but the card measured 0 — refusing to drop a table with data', n;
    END IF;
    RAISE NOTICE '080: affiliate_conversions verified empty (% rows)', n;
END $$;

-- The table's own FK (affiliate_user_id -> users(id) ON DELETE SET NULL) goes with it. Measured
-- before dropping: `pg_constraint.confrelid = 'public.affiliate_conversions'` has ZERO rows (nothing
-- references it), so no FK has to be dropped by name first and no CASCADE is needed.
DROP TABLE IF EXISTS affiliate_conversions;

DO $$
BEGIN
    IF to_regclass('public.affiliate_conversions') IS NOT NULL THEN
        RAISE EXCEPTION '080: affiliate_conversions still exists after DROP TABLE — refusing to continue';
    END IF;
    RAISE NOTICE '080: affiliate_conversions dropped (0 rows, reader route deleted, no product writer; the money ledger stays affiliate_commissions)';
END $$;
