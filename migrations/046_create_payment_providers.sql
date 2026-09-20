-- 046_create_payment_providers.sql
-- The checkout/checkout_handler.rs endpoints (/api/v1/payment-providers) query a
-- `payment_providers` table that was never created, so the capability was dead:
-- the LIST silently returned [] (unwrap_or_default) and the UPSERT always errored.
-- Per-tenant payment credentials entered by the admin in the panel (Provider Keys view).

CREATE TABLE IF NOT EXISTS payment_providers (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id     uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider_type varchar(64) NOT NULL,
    api_key       text,
    is_active     boolean NOT NULL DEFAULT true,
    created_at    timestamp with time zone NOT NULL DEFAULT now()
);

-- one row per (tenant, provider) — the admin UI deletes then inserts, and this keeps
-- the table honest if two saves race.
CREATE UNIQUE INDEX IF NOT EXISTS payment_providers_tenant_provider_uniq
    ON payment_providers (tenant_id, provider_type);
