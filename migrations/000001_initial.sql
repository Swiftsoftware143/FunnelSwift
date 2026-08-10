-- FunnelSwift Initial Schema

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Tenants
CREATE TABLE IF NOT EXISTS tenants (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(255) UNIQUE NOT NULL,
    logo TEXT,
    colors JSONB,
    settings JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Users
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'user',
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Plans
CREATE TABLE IF NOT EXISTS plans (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(255) UNIQUE NOT NULL,
    price DECIMAL(10,2) NOT NULL DEFAULT 0,
    max_leads INT,
    max_tags INT,
    has_dual_routing BOOLEAN NOT NULL DEFAULT false,
    has_multi_tenant BOOLEAN NOT NULL DEFAULT false,
    has_white_label BOOLEAN NOT NULL DEFAULT false,
    features JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Tenant Plan Subscriptions
CREATE TABLE IF NOT EXISTS tenant_plan_subscriptions (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    plan_id UUID NOT NULL REFERENCES plans(id),
    start_date TIMESTAMP NOT NULL DEFAULT NOW(),
    end_date TIMESTAMP,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Tag Groups
CREATE TABLE IF NOT EXISTS tag_groups (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    is_collapsible BOOLEAN NOT NULL DEFAULT true,
    sort_order INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Tags
CREATE TABLE IF NOT EXISTS tags (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    color VARCHAR(50),
    group_id UUID REFERENCES tag_groups(id) ON DELETE SET NULL,
    is_system BOOLEAN NOT NULL DEFAULT false,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Plan-Tag Mappings
CREATE TABLE IF NOT EXISTS plan_tag_mappings (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    tag_id UUID NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    source_software VARCHAR(255),
    target_software VARCHAR(255),
    commission_rates JSONB,
    allow_dual_routing BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Leads
CREATE TABLE IF NOT EXISTS leads (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    phone VARCHAR(50),
    company VARCHAR(255),
    source VARCHAR(100),
    status VARCHAR(50) DEFAULT 'new',
    stage VARCHAR(100) DEFAULT 'New',
    score INT,
    tags JSONB,
    custom_fields JSONB,
    notes TEXT,
    assigned_to UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Affiliates
CREATE TABLE IF NOT EXISTS affiliates (
    id VARCHAR(50) PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) NOT NULL,
    industry VARCHAR(255),
    commission_rate DECIMAL(5,2),
    tax_docs JSONB,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Affiliate Links
CREATE TABLE IF NOT EXISTS affiliate_links (
    id UUID PRIMARY KEY,
    affiliate_id VARCHAR(50) NOT NULL REFERENCES affiliates(id) ON DELETE CASCADE,
    target_software VARCHAR(255),
    tracking_code VARCHAR(255) UNIQUE NOT NULL,
    commission_rate DECIMAL(5,2),
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Affiliate Commissions
CREATE TABLE IF NOT EXISTS affiliate_commissions (
    id UUID PRIMARY KEY,
    affiliate_id VARCHAR(50) NOT NULL REFERENCES affiliates(id) ON DELETE CASCADE,
    lead_id UUID NOT NULL REFERENCES leads(id) ON DELETE CASCADE,
    amount DECIMAL(10,2) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    paid_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Target Software (for dual routing)
CREATE TABLE IF NOT EXISTS target_software (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    webhook_url TEXT NOT NULL,
    api_key TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Routing Log
CREATE TABLE IF NOT EXISTS routing_log (
    id UUID PRIMARY KEY,
    lead_id UUID NOT NULL REFERENCES leads(id) ON DELETE CASCADE,
    source_tenant UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    target_software UUID NOT NULL REFERENCES target_software(id) ON DELETE CASCADE,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    result JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Webhooks
CREATE TABLE IF NOT EXISTS webhooks (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    url TEXT NOT NULL,
    events JSONB NOT NULL,
    secret TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Webhook Delivery Log
CREATE TABLE IF NOT EXISTS webhook_delivery_log (
    id UUID PRIMARY KEY,
    webhook_id UUID NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL,
    request_body TEXT,
    response_body TEXT,
    delivered_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Tenant Settings
CREATE TABLE IF NOT EXISTS tenant_settings (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    key VARCHAR(255) NOT NULL,
    value JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, key)
);

-- System Settings
CREATE TABLE IF NOT EXISTS system_settings (
    id UUID PRIMARY KEY,
    key VARCHAR(255) UNIQUE NOT NULL,
    value JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Activity Log
CREATE TABLE IF NOT EXISTS activity_log (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id VARCHAR(255) NOT NULL,
    action VARCHAR(255) NOT NULL,
    entity_type VARCHAR(255),
    entity_id VARCHAR(255),
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_leads_tenant ON leads(tenant_id);
CREATE INDEX IF NOT EXISTS idx_leads_stage ON leads(tenant_id, stage);
CREATE INDEX IF NOT EXISTS idx_leads_source ON leads(tenant_id, source);
CREATE INDEX IF NOT EXISTS idx_tags_tenant ON tags(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tags_group ON tags(group_id);
CREATE INDEX IF NOT EXISTS idx_affiliates_tenant ON affiliates(tenant_id);
CREATE INDEX IF NOT EXISTS idx_activity_tenant ON activity_log(tenant_id, created_at);
CREATE INDEX IF NOT EXISTS idx_webhooks_tenant ON webhooks(tenant_id);
CREATE INDEX IF NOT EXISTS idx_plan_tag_mappings_plan ON plan_tag_mappings(plan_id);
CREATE INDEX IF NOT EXISTS idx_plan_tag_mappings_tag ON plan_tag_mappings(tag_id);
CREATE INDEX IF NOT EXISTS idx_routing_log_tenant ON routing_log(source_tenant);
CREATE INDEX IF NOT EXISTS idx_affiliate_commissions_affiliate ON affiliate_commissions(affiliate_id);

-- ============ SEED DATA ============

-- Seed Plans
INSERT INTO plans (id, name, slug, price, max_leads, max_tags, has_dual_routing, has_multi_tenant, has_white_label, features)
VALUES
    ('f0000000-0000-0000-0000-000000000001', 'Free', 'free', 0, 100, 20, false, false, false, '{"max_users": 1, "import_export": false, "analytics": false}'::jsonb),
    ('f0000000-0000-0000-0000-000000000002', 'Starter', 'starter', 29, 1000, 50, true, false, false, '{"max_users": 3, "import_export": true, "analytics": true}'::jsonb),
    ('f0000000-0000-0000-0000-000000000003', 'Pro', 'pro', 79, 10000, 200, true, true, false, '{"max_users": 10, "import_export": true, "analytics": true, "api_access": true}'::jsonb),
    ('f0000000-0000-0000-0000-000000000004', 'Enterprise', 'enterprise', 199, -1, -1, true, true, true, '{"max_users": -1, "import_export": true, "analytics": true, "api_access": true, "white_label": true, "dedicated_support": true}'::jsonb)
ON CONFLICT (slug) DO NOTHING;

-- First, create an admin tenant that will own the system tags
INSERT INTO tenants (id, name, slug) VALUES ('00000000-0000-0000-0000-000000000001', 'System', 'system') ON CONFLICT (slug) DO NOTHING;

-- Seed Tag Groups (belong to the admin user's tenant, referenced as system groups)
INSERT INTO tag_groups (id, tenant_id, name, is_collapsible, sort_order) VALUES 
    ('a0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', 'Source', true, 1),
    ('a0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Status', true, 2),
    ('a0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 'Events', true, 3),
    ('a0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000001', 'Services', true, 4),
    ('a0000000-0000-0000-0000-000000000005', '00000000-0000-0000-0000-000000000001', 'Engagement', true, 5),
    ('a0000000-0000-0000-0000-000000000006', '00000000-0000-0000-0000-000000000001', 'Custom', true, 6)
ON CONFLICT DO NOTHING;

-- Seed System Tags (shared across all tenants via is_system = true)
INSERT INTO tags (id, tenant_id, name, color, group_id, is_system) VALUES
    ('b0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', 'Referral', '#4CAF50', 'a0000000-0000-0000-0000-000000000001', true),
    ('b0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Website', '#2196F3', 'a0000000-0000-0000-0000-000000000001', true),
    ('b0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 'Cold Call', '#FF9800', 'a0000000-0000-0000-0000-000000000001', true),
    ('b0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000001', 'Email Campaign', '#9C27B0', 'a0000000-0000-0000-0000-000000000001', true),
    ('b0000000-0000-0000-0000-000000000005', '00000000-0000-0000-0000-000000000001', 'Social Media', '#E91E63', 'a0000000-0000-0000-0000-000000000001', true),
    ('b0000000-0000-0000-0000-000000000006', '00000000-0000-0000-0000-000000000001', 'Active', '#4CAF50', 'a0000000-0000-0000-0000-000000000002', true),
    ('b0000000-0000-0000-0000-000000000007', '00000000-0000-0000-0000-000000000001', 'Inactive', '#9E9E9E', 'a0000000-0000-0000-0000-000000000002', true),
    ('b0000000-0000-0000-0000-000000000008', '00000000-0000-0000-0000-000000000001', 'Won', '#4CAF50', 'a0000000-0000-0000-0000-000000000002', true),
    ('b0000000-0000-0000-0000-000000000009', '00000000-0000-0000-0000-000000000001', 'Lost', '#F44336', 'a0000000-0000-0000-0000-000000000002', true),
    ('b0000000-0000-0000-0000-00000000000a', '00000000-0000-0000-0000-000000000001', 'Webinar', '#00BCD4', 'a0000000-0000-0000-0000-000000000003', true),
    ('b0000000-0000-0000-0000-00000000000b', '00000000-0000-0000-0000-000000000001', 'Demo', '#FF5722', 'a0000000-0000-0000-0000-000000000003', true),
    ('b0000000-0000-0000-0000-00000000000c', '00000000-0000-0000-0000-000000000001', 'Meeting', '#795548', 'a0000000-0000-0000-0000-000000000003', true),
    ('b0000000-0000-0000-0000-00000000000d', '00000000-0000-0000-0000-000000000001', 'Proposal Sent', '#607D8B', 'a0000000-0000-0000-0000-000000000003', true),
    ('b0000000-0000-0000-0000-00000000000e', '00000000-0000-0000-0000-000000000001', 'Consulting', '#3F51B5', 'a0000000-0000-0000-0000-000000000004', true),
    ('b0000000-0000-0000-0000-00000000000f', '00000000-0000-0000-0000-000000000001', 'Marketing', '#FF4081', 'a0000000-0000-0000-0000-000000000004', true),
    ('b0000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000001', 'Sales', '#448AFF', 'a0000000-0000-0000-0000-000000000004', true),
    ('b0000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-000000000001', 'Support', '#69F0AE', 'a0000000-0000-0000-0000-000000000004', true),
    ('b0000000-0000-0000-0000-000000000012', '00000000-0000-0000-0000-000000000001', 'Hot', '#F44336', 'a0000000-0000-0000-0000-000000000005', true),
    ('b0000000-0000-0000-0000-000000000013', '00000000-0000-0000-0000-000000000001', 'Warm', '#FF9800', 'a0000000-0000-0000-0000-000000000005', true),
    ('b0000000-0000-0000-0000-000000000014', '00000000-0000-0000-0000-000000000001', 'Cold', '#2196F3', 'a0000000-0000-0000-0000-000000000005', true),
    ('b0000000-0000-0000-0000-000000000015', '00000000-0000-0000-0000-000000000001', 'New Lead', '#00E676', 'a0000000-0000-0000-0000-000000000005', true),
    ('b0000000-0000-0000-0000-000000000016', '00000000-0000-0000-0000-000000000001', 'Follow Up', '#AA00FF', 'a0000000-0000-0000-0000-000000000005', true)
ON CONFLICT DO NOTHING;
