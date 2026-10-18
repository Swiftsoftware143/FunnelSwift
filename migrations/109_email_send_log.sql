-- 109_email_send_log.sql — the HISTORY behind "did that credentials mail go out?"
--
-- `admin_settings.email_last_send` holds ONE record and every later mail overwrites it, so a
-- welcome/credentials mail the provider refused at signup is erased the moment any other message
-- succeeds. An operator asking "did this customer ever get their password?" had nothing to read.
-- This table is append-only, written from the ONE place every send outcome is recorded
-- (`email::record_last_send` / `email::record_withheld`), so a new send path inherits it.
--
-- `withheld` is a first-class status, not a failure: a fleet-harness/reserved recipient is never
-- attempted at all (no provider request), and recording that as a refusal would be the misreport
-- the sibling apps fixed on 2026-10-14.
CREATE TABLE IF NOT EXISTS email_send_log (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    to_email   text        NOT NULL,
    subject    text,
    status     text        NOT NULL CHECK (status IN ('sent', 'failed', 'withheld')),
    detail     text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS email_send_log_created_idx ON email_send_log (created_at DESC);
