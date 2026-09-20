-- 047: checkout session store for FunnelSwift.
--
-- The thank-you pages (www/thank-you.html, www-app/thank-you.html) fetch
-- /api/v1/checkout/session/<id>. Before this migration FunnelSwift had no
-- checkout session store at all — checkout_handler was a pure stub — so the
-- post-payment page could never show real data. Schema mirrors the other
-- fleet apps (missedcallrespondr/ADASwift) but is tenant-scoped, matching
-- payment_providers.tenant_id.

CREATE TABLE IF NOT EXISTS checkout_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    user_id UUID,
    provider_type VARCHAR(32) NOT NULL,
    provider_session_id VARCHAR(255) NOT NULL DEFAULT '',
    purchasable_type VARCHAR(64) NOT NULL,
    purchasable_id UUID,
    amount NUMERIC(12,2) NOT NULL DEFAULT 0,
    currency VARCHAR(3) NOT NULL DEFAULT 'USD',
    status VARCHAR(32) NOT NULL DEFAULT 'pending',
    metadata JSONB NOT NULL DEFAULT '{}',
    webhook_event_id VARCHAR(255),
    webhook_received_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_tenant_id
    ON checkout_sessions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_checkout_sessions_provider_session
    ON checkout_sessions(provider_type, provider_session_id);
CREATE INDEX IF NOT EXISTS idx_checkout_sessions_status
    ON checkout_sessions(status);
