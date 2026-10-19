-- A workspace can ASK to be moved onto a paid plan (2026-10-19, cron `default`).
--
-- WHY THIS EXISTS, measured live: this install has no payment provider configured, so
-- `POST /api/v1/checkout/create` refuses every purchase with 503
-- `payment_provider_not_configured`. The Upgrade screen then told the customer "Upgrades are not
-- open on this workspace yet — nothing was charged" and, on its other failure arm, told them to
-- "open a support ticket" — a screen this app does not have. Nothing was written anywhere, so a
-- customer who wanted to PAY left no trace the operator could act on.
--
-- The row is the record: the customer files it from the Upgrade screen and sees its live state
-- there; the platform operator reads the same rows in the admin arm and moves the status on.
--
-- Additive and idempotent: a replayed boot is a no-op and nothing existing is touched.
CREATE TABLE IF NOT EXISTS plan_requests (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id      UUID,
    requested_by TEXT NOT NULL DEFAULT '',
    plan_slug    TEXT NOT NULL,
    plan_name    TEXT NOT NULL DEFAULT '',
    note         TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'open',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- The customer's own list: newest first, per workspace.
CREATE INDEX IF NOT EXISTS plan_requests_tenant_idx
    ON plan_requests (tenant_id, created_at DESC);

-- The duplicate guard: one OPEN request per workspace+plan (an open row is what the create arm
-- looks for before inserting).
CREATE INDEX IF NOT EXISTS plan_requests_open_idx
    ON plan_requests (tenant_id, plan_slug) WHERE status IN ('open', 'contacted');
