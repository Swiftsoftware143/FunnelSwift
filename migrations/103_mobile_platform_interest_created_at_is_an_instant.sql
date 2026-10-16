-- 103_mobile_platform_interest_created_at_is_an_instant.sql
-- The row-reader for this table (handlers/mobile_notify_handler.rs::list_mobile_interest) failed its
-- first LIVE call with
--
--   error occurred while decoding column 3: mismatched types; Rust type
--   `chrono::datetime::DateTime<Utc>` (as SQL type `TIMESTAMPTZ`) is not compatible with SQL type
--   `TIMESTAMP`
--
-- measured on the freshly deployed console screen (2026-10-16, GET /api/v1/admin/mobile-interest
-- answered 500). Migration 102 declared `created_at timestamp` — the column holds an INSTANT, and the
-- database's own session timezone is UTC (`show timezone` = UTC), so the naive values are UTC.
-- Making the column say so (timestamptz) is the honest fix: every reader then gets the instant with
-- its offset, and the console renders the operator's local time instead of silently treating a UTC
-- string as local.
--
-- `USING … AT TIME ZONE 'UTC'` is the correct conversion for values that were written by NOW() under
-- a UTC session — not a guess: it was read from the live database before this file was written.
ALTER TABLE mobile_platform_interest
    ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
