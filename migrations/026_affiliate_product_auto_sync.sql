-- Migration 026: Affiliate Product Auto-Sync
-- Adds columns to support automatic syncing of plans to affiliate_products
-- and seeds default product categories.

-- Add owner_name (who owns this product/service)
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS owner_name VARCHAR(255) NOT NULL DEFAULT 'SwiftSoftware';

-- Add product_type ('software' or 'service')
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS product_type VARCHAR(50) NOT NULL DEFAULT 'software';

-- Add plan_id FK reference back to source plan
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS plan_id UUID REFERENCES plans(id) ON DELETE SET NULL;

-- Add source_app tracking (which app the plan belongs to)
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS source_app VARCHAR(100);

-- Seed default product categories if table is empty, using a selector that works
-- with or without existing data. We INSERT per-name with NOT EXISTS guard.

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'FunnelSwift Plans', 'funnelswift-plans', 'FunnelSwift subscription plans', 1, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'funnelswift-plans')
LIMIT 1;

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'Kinetic Cards', 'kinetic-cards', 'Kinetic digital business cards', 2, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'kinetic-cards')
LIMIT 1;

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'IncentiveSwift Plans', 'incentiveswift-plans', 'IncentiveSwift incentive plans', 3, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'incentiveswift-plans')
LIMIT 1;

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'MultiDirectory Plans', 'multidirectory-plans', 'MultiDirectory directory plans', 4, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'multidirectory-plans')
LIMIT 1;

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'CoreSwift Plans', 'coreswift-plans', 'CoreSwift core service plans', 5, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'coreswift-plans')
LIMIT 1;

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'WorkflowSwift Plans', 'workflowswift-plans', 'WorkflowSwift workflow automation plans', 6, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'workflowswift-plans')
LIMIT 1;

INSERT INTO product_categories (id, tenant_id, name, slug, description, sort_order, is_active)
SELECT gen_random_uuid(), t.id, 'AdaSwift Plans', 'adaswift-plans', 'AdaSwift AI service plans', 7, true
FROM tenants t
WHERE NOT EXISTS (SELECT 1 FROM product_categories WHERE slug = 'adaswift-plans')
LIMIT 1;
