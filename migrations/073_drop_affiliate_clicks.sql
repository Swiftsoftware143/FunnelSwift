-- 073: DROP `affiliate_clicks` — the affiliate click table nothing could ever populate or display
-- (kanban t_6a7e77e1; residue of the /api/v1/track-click deletion, t_813d51d4).
--
-- MEASURED on the live `funnelswift` database before this migration (2026-10-01):
--   * `affiliate_clicks` = **0 rows**. It has never held one.
--   * `grep -rn affiliate_clicks src/` = comments only: no reader, no writer, no route, no query.
--   * The one route that ever pointed here, `GET /api/v1/track-click`, was a no-op — `Query(_)`
--     discarded, `State(_)` unused, a constant `200 {"tracked": true}` and not one row written — and
--     was deleted (commit 2653b04) after a fleet-wide caller census found zero callers (every app's
--     src/, every served www*/ root, nginx, n8n) and zero consumers of the table.
--   * Attribution in this product is resolved at SIGNUP: `?ref=<code>` ->
--     `affiliate_links.tracking_code` -> `leads.created_by` (public_signup_handler /
--     affiliate_referral_handler). The affiliate portal dashboard sums `affiliate_commissions`
--     (leads + earnings/movements); the real click analytics are the Kinetic card tracker
--     `POST /card/:id/track` -> `kinetic_card_events` / `kinetic_card_daily_stats.clicks`.
-- So there is no caller to serve and no reader to display: an empty table with no writer is a false
-- oracle — every from-zero or schema audit reports it as "a feature with data waiting" (it was
-- listed as an owned table in ARCHITECTURE.md). The programme keeps every table that carries
-- affiliates, links, products, conversions and commissions; this one carries nothing.
--
-- The inbound FK `affiliate_conversions.click_id -> affiliate_clicks(id)` (declared by 062) goes with
-- the table. It is dropped BY NAME first so this migration never needs `CASCADE` — nothing else in
-- the database depends on `affiliate_clicks` (measured: `pg_constraint.confrelid` = exactly one row,
-- that FK; no views, no rules). `affiliate_conversions.click_id` stays as a nullable legacy column:
-- no code writes it (there is no INSERT into `affiliate_conversions` anywhere) and the portal's
-- conversion list reads it as NULL.
--
-- 021 (`CREATE TABLE affiliate_clicks`) is NOT edited — an applied migration is history. A rollback
-- is `CREATE TABLE affiliate_clicks (...)` copied from 021; the table is empty, so nothing is lost.

ALTER TABLE affiliate_conversions
    DROP CONSTRAINT IF EXISTS affiliate_conversions_click_id_fkey;

DROP TABLE IF EXISTS affiliate_clicks;

DO $$
BEGIN
    IF to_regclass('public.affiliate_clicks') IS NOT NULL THEN
        RAISE EXCEPTION '073: affiliate_clicks still exists after DROP TABLE — refusing to continue';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'affiliate_conversions_click_id_fkey'
           AND conrelid = 'public.affiliate_conversions'::regclass)
    THEN
        RAISE EXCEPTION '073: affiliate_conversions.click_id still points at a dropped table';
    END IF;
    RAISE NOTICE '073: affiliate_clicks dropped (0 rows, no reader, no writer; the FK from '
                 'affiliate_conversions went with it, its click_id column kept as nullable legacy)';
END $$;
