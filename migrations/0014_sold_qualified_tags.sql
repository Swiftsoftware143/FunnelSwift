-- Phase 1: SOLD + QUALIFIED system tags and tag rules
-- These tags live under the System tenant for use across all workspaces

-- Create the "Qualified" system tag under Status group
INSERT INTO tags (id, tenant_id, group_id, name, color, is_system)
VALUES ('15698a9a-67fe-5bf1-9aac-1dcd7a1ccd9e', '00000000-0000-0000-0000-000000000001', 'a0000000-0000-0000-0000-000000000002', 'Qualified', '#4CAF50', true)
ON CONFLICT (id) DO NOTHING;

-- Create the "Sold" system tag under Status group
INSERT INTO tags (id, tenant_id, group_id, name, color, is_system)
VALUES ('3b008e4a-dbc8-5558-8762-2e1787ec7c2c', '00000000-0000-0000-0000-000000000001', 'a0000000-0000-0000-0000-000000000002', 'Sold', '#FF9800', true)
ON CONFLICT (id) DO NOTHING;

-- Tag rule: When Sold is assigned → remove Qualified
INSERT INTO tag_rules (id, tenant_id, name, description, trigger_tag_id, action_type, action_tag_id, target_app, is_active)
VALUES (
    'dcdd1042-9a01-5797-8e14-d2653825c74c',
    '00000000-0000-0000-0000-000000000001',
    'Sold removes Qualified',
    'When a lead gets the Sold tag, auto-remove the Qualified tag',
    '3b008e4a-dbc8-5558-8762-2e1787ec7c2c',  -- Sold
    'remove_tag',
    '15698a9a-67fe-5bf1-9aac-1dcd7a1ccd9e',  -- Qualified
    'funnelswift',
    true
)
ON CONFLICT (id) DO NOTHING;
