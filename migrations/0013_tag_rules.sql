-- Tag Rules: conditional logic for tag auto-management
-- When a condition tag is added to a lead, action tags are added/removed automatically

CREATE TABLE IF NOT EXISTS tag_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    trigger_tag_id UUID NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    action_type VARCHAR(50) NOT NULL CHECK (action_type IN ('add_tag', 'remove_tag', 'replace')),
    action_tag_id UUID REFERENCES tags(id) ON DELETE CASCADE,
    target_app VARCHAR(100) DEFAULT 'funnelswift',
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_tag_rules_tenant ON tag_rules(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tag_rules_trigger ON tag_rules(trigger_tag_id);

-- Tag change log: records all tag mutations per lead (for audit + sync)
CREATE TABLE IF NOT EXISTS tag_change_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    lead_id UUID NOT NULL,
    added_tags JSONB DEFAULT '[]'::jsonb,
    removed_tags JSONB DEFAULT '[]'::jsonb,
    triggered_by VARCHAR(100) DEFAULT 'manual',
    synced_to_core BOOLEAN NOT NULL DEFAULT false,
    synced_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_tag_change_lead ON tag_change_log(lead_id);
CREATE INDEX IF NOT EXISTS idx_tag_change_unsynced ON tag_change_log(tenant_id, synced_to_core) WHERE synced_to_core = false;
