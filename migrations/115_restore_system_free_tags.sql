-- Migration 115: RESTORE the seven system "free plan" tags and their routing links.
--
-- MEASURED DEFECT (2026-10-22). The tag -> free-account executor
-- (`src/app_provision.rs::provision_for_added_tags`) resolves a mapping with:
--
--   SELECT ... FROM tags
--    WHERE name = ANY($1) AND (tenant_id = $2 OR is_system = true)
--      AND provisions_account = true AND source_app IS NOT NULL AND source_app <> 'funnelswift'
--
-- LIVE state before this file:
--   * `SELECT count(*) FROM tags` = 3     -> NOT ONE system tag existed (migration 043 seeds seven)
--   * `SELECT count(system_tag_id) FROM affiliate_products` = 0 of 7
--   * `SELECT count(*) FROM provisioning_log` = 0
-- so applying a tag to a lead could never mint an account: David's onboarding path was dead, and the
-- admin "system tag" dropdown (affiliate_product_handler::list_system_tags, `is_system = true`) was empty.
--
-- This file re-creates the SAME seven tags migration 043 created, under the SAME System tenant, with
-- the SAME per-app mapping migrations 067 and 096 derived, and re-links the seven free affiliate
-- products to them. It invents nothing: every value below is read from 043 (names/colours),
-- 067 (source_app + plan_slug + plan_id rule) and 096 (provisions_account = source_app/plan_slug set).
--
-- Idempotent: every statement is a guarded INSERT or an UPDATE that is a no-op once correct, so the
-- runner may re-execute it on any boot.

-- ── 0. The System tenant is the documented home (043 header, 067 header) ────────────────────────
INSERT INTO tenants (id, name, slug)
SELECT '00000000-0000-0000-0000-000000000001', 'System', 'system'
WHERE NOT EXISTS (SELECT 1 FROM tenants WHERE id = '00000000-0000-0000-0000-000000000001');

-- ── 1. The seven system tags, exactly the names and colours 043 seeded ──────────────────────────
INSERT INTO tags (id, tenant_id, name, color, is_system)
SELECT gen_random_uuid(), '00000000-0000-0000-0000-000000000001', v.name, v.color, true
FROM (VALUES
        ('FunnelSwift — Capture Free',       '#2563eb'),
        ('FunnelSwift — Kinetic Free',        '#7c3aed'),
        ('CoreSwift — Free',                  '#059669'),
        ('WorkflowSwift — Free',              '#0ea5e9'),
        ('IncentiveSwift — Free',             '#f59e0b'),
        ('ADASwift — Free',                    '#dc2626'),
        ('MissedCall Respondr — Free',         '#475569')
     ) AS v(name, color)
WHERE NOT EXISTS (SELECT 1 FROM tags t WHERE t.name = v.name AND t.is_system = true);

-- ── 2. The mapping each tag carries ────────────────────────────────────────────────────────────
-- Sibling apps: `source_app` + `plan_slug = 'free'` (067's backfill rule for the five sibling tags).
UPDATE tags
   SET source_app = 'coreswift',           plan_slug = 'free'
 WHERE name = 'CoreSwift — Free' AND is_system = true;
UPDATE tags
   SET source_app = 'workflowswift',       plan_slug = 'free'
 WHERE name = 'WorkflowSwift — Free' AND is_system = true;
UPDATE tags
   SET source_app = 'incentiveswift',      plan_slug = 'free'
 WHERE name = 'IncentiveSwift — Free' AND is_system = true;
UPDATE tags
   SET source_app = 'adaswift',            plan_slug = 'free'
 WHERE name = 'ADASwift — Free' AND is_system = true;
UPDATE tags
   SET source_app = 'missedcallrespondr',  plan_slug = 'free'
 WHERE name = 'MissedCall Respondr — Free' AND is_system = true;

-- FunnelSwift's own two: the plan IS in this database, so link it for real (067's rule: the tag name
-- is '<App> — ' || plan.name, and `slug` is the plan's own slug).
UPDATE tags t
   SET source_app = 'funnelswift',
       plan_id    = p.id,
       plan_slug  = p.slug
  FROM plans p
 WHERE t.is_system
   AND t.name = 'FunnelSwift — ' || p.name
   AND p.slug IN ('capture-free', 'kinetic-free');

-- The per-tag switch, set by the same predicate 096 used.
UPDATE tags
   SET provisions_account = true
 WHERE is_system = true
   AND source_app IS NOT NULL
   AND plan_slug IS NOT NULL
   AND provisions_account IS DISTINCT FROM true;

-- ── 3. Re-link the seven free affiliate products to their tag ─────────────────────────────────
-- 067 reads this column INTO tags.source_app, so with it NULL the chain was severed at both ends.
-- Sibling products link by the app they name.
UPDATE affiliate_products p
   SET system_tag_id = t.id
  FROM tags t
 WHERE p.system_tag_id IS NULL
   AND p.source_app IS NOT NULL
   AND t.is_system
   AND t.source_app = p.source_app
   AND t.name = CASE p.source_app
                  WHEN 'coreswift'          THEN 'CoreSwift — Free'
                  WHEN 'workflowswift'      THEN 'WorkflowSwift — Free'
                  WHEN 'incentiveswift'     THEN 'IncentiveSwift — Free'
                  WHEN 'adaswift'           THEN 'ADASwift — Free'
                  WHEN 'missedcallrespondr' THEN 'MissedCall Respondr — Free'
                END;

-- FunnelSwift's two link through the plan they give away.
UPDATE affiliate_products p
   SET system_tag_id = t.id
  FROM tags t
 WHERE p.system_tag_id IS NULL
   AND p.plan_id IS NOT NULL
   AND t.is_system
   AND t.plan_id = p.plan_id;
