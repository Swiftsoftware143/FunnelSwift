-- Pause switch for a shared funnel page.
--
-- An operator can stop a live opt-in page taking responses WITHOUT deleting the funnel or
-- changing its link (the link may be printed, pasted in a chat, or already in a paid ad). While
-- `is_active` is false the public page shows a closed notice instead of the form, and
-- `POST /funnel/:slug/submit` refuses a submission that a cached page still sends.
--
-- Default TRUE so every funnel that exists today keeps collecting leads untouched, and
-- IF NOT EXISTS so the boot runner (which re-executes unrecorded files) is idempotent.
ALTER TABLE funnels ADD COLUMN IF NOT EXISTS is_active boolean NOT NULL DEFAULT true;
