-- Migration 043: Seed free-plan affiliate products + their system tags.
--
-- David's model: each Swift product's FREE tier is an affiliate product. The system
-- tag linked to each product is the routing signal ("this lead acquired the free plan
-- of product X"); upsells happen in-app and credit the referring affiliate via the
-- upgrade-event endpoint.
--
-- FunnelSwift has TWO free plans: Capture Free (lead capture) and Kinetic Free.
-- All products live under the System tenant (deterministic), the same home as
-- cross-app-synced plans and the Sold/Qualified system tags.

-- ── 1. System tags (is_system = true) ─────────────────────────────────────
INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'FunnelSwift — Capture Free', '#2563eb', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'FunnelSwift — Capture Free' AND is_system = true);

INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'FunnelSwift — Kinetic Free', '#7c3aed', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'FunnelSwift — Kinetic Free' AND is_system = true);

INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'CoreSwift — Free', '#059669', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'CoreSwift — Free' AND is_system = true);

INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'WorkflowSwift — Free', '#0ea5e9', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'WorkflowSwift — Free' AND is_system = true);

INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'IncentiveSwift — Free', '#f59e0b', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'IncentiveSwift — Free' AND is_system = true);

INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'ADASwift — Free', '#dc2626', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'ADASwift — Free' AND is_system = true);

INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'MissedCall Respondr — Free', '#475569', true
WHERE NOT EXISTS (SELECT 1 FROM tags WHERE name = 'MissedCall Respondr — Free' AND is_system = true);

-- ── 2. Affiliate products linked to those tags ────────────────────────────
-- FunnelSwift free plans carry plan_id (capture-free / kinetic-free); others use source_app.

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, plan_id, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'Capture Free',
       'FunnelSwift lead capture — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'funnelswift',
       'f0000000-0000-0000-0000-000000000001',
       (SELECT id FROM tags WHERE name = 'FunnelSwift — Capture Free' AND is_system = true LIMIT 1),
       'funnelswift-capture-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'Capture Free' AND source_app = 'funnelswift');

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, plan_id, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'Kinetic Free',
       'FunnelSwift Kinetic digital cards — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'funnelswift',
       'f0000000-0000-0000-0000-000000000002',
       (SELECT id FROM tags WHERE name = 'FunnelSwift — Kinetic Free' AND is_system = true LIMIT 1),
       'funnelswift-kinetic-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'Kinetic Free' AND source_app = 'funnelswift');

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'CoreSwift Free',
       'CoreSwift CRM — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'coreswift',
       (SELECT id FROM tags WHERE name = 'CoreSwift — Free' AND is_system = true LIMIT 1),
       'coreswift-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'CoreSwift Free' AND source_app = 'coreswift');

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'WorkflowSwift Free',
       'WorkflowSwift automation — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'workflowswift',
       (SELECT id FROM tags WHERE name = 'WorkflowSwift — Free' AND is_system = true LIMIT 1),
       'workflowswift-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'WorkflowSwift Free' AND source_app = 'workflowswift');

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'IncentiveSwift Free',
       'IncentiveSwift campaigns & loyalty — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'incentiveswift',
       (SELECT id FROM tags WHERE name = 'IncentiveSwift — Free' AND is_system = true LIMIT 1),
       'incentiveswift-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'IncentiveSwift Free' AND source_app = 'incentiveswift');

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'ADASwift Free',
       'ADASwift accessibility — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'adaswift',
       (SELECT id FROM tags WHERE name = 'ADASwift — Free' AND is_system = true LIMIT 1),
       'adaswift-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'ADASwift Free' AND source_app = 'adaswift');

INSERT INTO affiliate_products (id, tenant_id, name, description, price, default_commission_rate, is_active, is_third_party, product_type, owner_name, source_app, system_tag_id, slug)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', 'MissedCall Respondr Free',
       'MissedCall Respondr — free plan', 0, 20.0, true, false, 'software', 'SwiftSoftware', 'missedcall',
       (SELECT id FROM tags WHERE name = 'MissedCall Respondr — Free' AND is_system = true LIMIT 1),
       'missedcall-free'
WHERE NOT EXISTS (SELECT 1 FROM affiliate_products WHERE name = 'MissedCall Respondr Free' AND source_app = 'missedcall');
