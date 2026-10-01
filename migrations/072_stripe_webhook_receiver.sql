-- 072: Stripe — the receiver that makes an in-app upgrade actually land
--
-- David, 2026-10-01: *"i will use stripe for now"*, after the gap was measured on the live app:
--   POST /api/v1/checkout/create -> 503 {"configured":false,"error":"payment_provider_not_configured"}
--   …and with a provider configured it answered 501 "checkout_not_implemented";
--   and NO inbound payment receiver was registered anywhere.
-- So a free customer could not upgrade themselves at all, which meant the affiliate credit path
-- (built and proven in 753fa2f) could never fire, because nothing ever called it.

-- A provider needs a signing secret to verify its webhooks. Same at-rest rule as `api_key`: this
-- column only ever holds `enc:v1:` ciphertext (src/security/provider_key_crypto.rs). A plaintext
-- secret is refused by the DATABASE, not merely by the code, so a future writer cannot leak one.
ALTER TABLE payment_providers ADD COLUMN IF NOT EXISTS webhook_secret text;

DO $$
BEGIN
    ALTER TABLE payment_providers ADD CONSTRAINT payment_providers_webhook_secret_encrypted
        CHECK (webhook_secret IS NULL OR webhook_secret = '' OR webhook_secret LIKE 'enc:v1:%');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

-- Every delivery is recorded BEFORE it is acted on, and the provider's event id is UNIQUE: a retried
-- delivery (Stripe retries for up to 3 days) can never move a plan twice. This is also the audit
-- trail an operator needs when a customer says "I paid and nothing happened" — the refusal reason
-- lives here, not only in a log line.
CREATE TABLE IF NOT EXISTS payment_webhook_events (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_type text NOT NULL DEFAULT 'stripe',
    event_id      text NOT NULL,          -- the provider's own event id (the idempotency key)
    event_type    text NOT NULL,
    -- processed | ignored | duplicate | signature_failed | not_configured | error
    status        text NOT NULL,
    http_status   int,
    error_message text,
    tenant_id     uuid,
    payload       jsonb NOT NULL DEFAULT '{}'::jsonb,
    received_at   timestamptz NOT NULL DEFAULT NOW(),
    processed_at  timestamptz
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_pwe_provider_event
    ON payment_webhook_events (provider_type, event_id);
CREATE INDEX IF NOT EXISTS idx_pwe_received ON payment_webhook_events (received_at DESC);
CREATE INDEX IF NOT EXISTS idx_pwe_status   ON payment_webhook_events (status, received_at DESC);

COMMENT ON TABLE payment_webhook_events IS
'One row per payment-provider delivery. UNIQUE (provider_type, event_id) is the idempotency guard; status carries the refusal reason so a customer dispute can be answered from the database.';
