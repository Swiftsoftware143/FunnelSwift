-- 133_private_email_attachments.sql
-- THE FILE ITSELF (ported from ADASwift's 029 / CoreSwift's 147, kanban t_f919924f).
--
-- Until this migration the receiver accepted a delivery with attachments, stored the MESSAGE with
-- has_attachments = true and counted the file's bytes into private_email_messages.size_bytes, then
-- DROPPED the bytes. A customer received an invoice, a screenshot or a signed PDF and the inbox
-- showed a message with nothing to open — silent data loss from their point of view. The app's own
-- branded mail made the same hole visible on every send: the logo is an inline `cid:` part, and the
-- reader had no image to draw.
--
-- A file part is now its own row, keyed to the message it travelled with and carrying the account
-- it belongs to (so a download can be tenancy-scoped without trusting the message id alone).
--
--   * is_inline = true  — the body refers to it as `cid:<name>`; the reader renders it in place.
--   * is_inline = false — a real attachment; the reader offers it as a download.
--
-- STORAGE IS COUNTED ONCE. The bytes are already inside the parent message's `size_bytes` (that is
-- what the meter, the banner and the 100% refusal read), so nothing sums this table: it is the
-- PAYLOAD, not a second meter.
--
-- ON DELETE CASCADE: destroying a message destroys the files it carried with it.

CREATE TABLE IF NOT EXISTS private_email_attachments (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    message_id   uuid NOT NULL REFERENCES private_email_messages(id) ON DELETE CASCADE,
    filename     text NOT NULL,
    content_type varchar(255) NOT NULL DEFAULT 'application/octet-stream',
    content_id   text,
    is_inline    boolean NOT NULL DEFAULT false,
    size_bytes   integer NOT NULL DEFAULT 0,
    data         bytea NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_private_email_attachments_message
    ON private_email_attachments (tenant_id, message_id);
CREATE INDEX IF NOT EXISTS idx_private_email_attachments_inline
    ON private_email_attachments (message_id) WHERE is_inline = true;

-- An inline part is fetched by (message, content_id) when the body is rewritten.
CREATE INDEX IF NOT EXISTS idx_private_email_attachments_content_id
    ON private_email_attachments (message_id, content_id) WHERE content_id IS NOT NULL;
