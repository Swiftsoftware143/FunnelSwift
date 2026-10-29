-- Persist WHAT each metered card scan produced (cron 2026-10-29).
--
-- THE GAP THIS CLOSES: `ocr_scans` (migration 045) recorded only `tenant_id` and `created_at`, so
-- `max_ocr_scans` — a SOLD, plan-gated, metered feature — charged the customer an allowance unit
-- and left no trace of what the unit bought. The plan strip could therefore show "Card scans 7/10"
-- with nothing anywhere to explain the seven, and a scan whose result was lost (page reload, the
-- degraded "no text" path) still cost a unit. This adds the reading itself to the same row the
-- counter already sums, so the number the customer sees and the list behind it come from one place.
--
-- `ocr_ok` is false for the degraded path (`call_ocr_service` unreachable / no usable text): the
-- attempt is still metered — that is what 045 decided and this migration does not change it — but
-- the row now says whether a reading came back, so the screen can label it honestly.
--
-- Idempotent on purpose: the boot runner applies unrecorded files, and re-running this against an
-- already-migrated database must not fail the boot.
ALTER TABLE ocr_scans ADD COLUMN IF NOT EXISTS fields JSONB;
ALTER TABLE ocr_scans ADD COLUMN IF NOT EXISTS ocr_ok BOOLEAN NOT NULL DEFAULT false;
