-- Durable retry queue for captures that never reached the CoreSwift hub.
--
-- Why this table exists: `spawn_lead_push` is fire-and-forget. When the hub answered 502
-- (measured live 2026-11-25 00:01-00:08, six consecutive POSTs to /api/external/contacts), the
-- lead stayed in FunnelSwift and the only trace was a log line — nothing ever retried, so the
-- customer's capture never appeared in their CRM. The capture itself is never lost, but the
-- hand-off was.
--
-- The row IS the queue (same shape as `webhook_delivery_log`, which has run in production since
-- kanban t_431faa99): the request path inserts the hub body it could not deliver, and
-- `coreswift::spawn_push_retry_worker` claims due rows with FOR UPDATE SKIP LOCKED, re-posts
-- them, and reschedules with a growing delay until `max_attempts`.
--
-- The BODY is stored, not the credentials: the hub base URL and the tenant's key are resolved
-- again at send time, so a rotated key or a changed base URL is honoured by the retry rather
-- than being frozen at capture time.

CREATE TABLE IF NOT EXISTS coreswift_push_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    payload JSONB NOT NULL,
    -- pending | sending | sent | failed
    status TEXT NOT NULL DEFAULT 'pending',
    attempt INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 6,
    last_error TEXT,
    next_retry_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ
);

-- The sweep's own query: due pending rows, oldest first.
CREATE INDEX IF NOT EXISTS coreswift_push_queue_due
    ON coreswift_push_queue (status, next_retry_at);

CREATE INDEX IF NOT EXISTS coreswift_push_queue_tenant
    ON coreswift_push_queue (tenant_id, created_at DESC);
