-- Web-to-Lead configuration table
CREATE TABLE IF NOT EXISTS web_to_lead_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL DEFAULT 'Default Form',
    is_active BOOLEAN NOT NULL DEFAULT true,
    tag_id UUID REFERENCES tags(id) ON DELETE SET NULL,
    tag_name VARCHAR(255),
    default_source VARCHAR(255) DEFAULT 'Web Form',
    field_mapping JSONB DEFAULT '{}'::jsonb,
    allowed_domains TEXT[] DEFAULT '{}',
    rate_limit_per_hour INTEGER DEFAULT 100,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_web_to_lead_tenant ON web_to_lead_configs(tenant_id);

-- Track incoming leads per config
CREATE TABLE IF NOT EXISTS web_to_lead_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    config_id UUID NOT NULL REFERENCES web_to_lead_configs(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    requester_ip VARCHAR(45),
    origin_domain VARCHAR(255),
    field_count INTEGER DEFAULT 0,
    lead_id UUID,
    status VARCHAR(50) NOT NULL DEFAULT 'received',
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_web_to_lead_logs_config ON web_to_lead_logs(config_id);
CREATE INDEX IF NOT EXISTS idx_web_to_lead_logs_tenant ON web_to_lead_logs(tenant_id);
