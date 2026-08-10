-- Portfolio Companies table
CREATE TABLE IF NOT EXISTS portfolio_companies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    settings JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_portfolio_companies_tenant ON portfolio_companies(tenant_id);

-- Add portfolio_company_id to existing target_software table
ALTER TABLE target_software ADD COLUMN IF NOT EXISTS portfolio_company_id UUID REFERENCES portfolio_companies(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_target_software_company ON target_software(portfolio_company_id);
ALTER TABLE target_software ADD COLUMN IF NOT EXISTS events TEXT[] DEFAULT '{}';
