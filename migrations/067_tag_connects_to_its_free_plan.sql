-- Every affiliate tag must connect to ITS free plan (David, 2026-09-29).
--
-- MEASURED BEFORE THIS MIGRATION: the connection did not exist anywhere.
--   * `tags` had no plan reference at all — id, tenant_id, name, color, group_id, is_system,
--     metadata, created_at, updated_at. `metadata` was empty on every row.
--   * `affiliate_products.plan_id` existed and was NULL on **all 5** products.
--   * FunnelSwift had no product behind either of its two free tags.
-- So a tag could not name the plan it represents, and nothing could act on that link — which is
-- exactly what "when a person gets tagged they should automatically get an account for them" needs.
--
-- WHY THREE COLUMNS AND NOT ONE:
--   * `plan_id` is a real foreign key, and it can only point at FunnelSwift's OWN plans (Capture Free,
--     Kinetic Free). A tag for another app cannot FK to a row in a different database, so a bare
--     plan_id would be null-and-useless for 5 of the 7 tags.
--   * `source_app` + `plan_slug` carry the reference for the sibling tags: which app owns the plan, and
--     the plan's slug inside that app. That is the pair a provisioning call needs.
-- Additive only: three nullable columns, nothing dropped, nothing given a NOT NULL.

ALTER TABLE tags ADD COLUMN IF NOT EXISTS source_app varchar(100);
ALTER TABLE tags ADD COLUMN IF NOT EXISTS plan_slug  varchar(255);
ALTER TABLE tags ADD COLUMN IF NOT EXISTS plan_id    uuid REFERENCES plans(id) ON DELETE SET NULL;

-- ── backfill the five sibling tags from the product that already points at them ──────────────────
-- `affiliate_products.system_tag_id` is the existing, working link (all 5 products carry it), so the
-- app that owns each tag is already recorded. This reads that rather than parsing the tag NAME, which
-- would break the moment somebody renamed a tag.
UPDATE tags t
   SET source_app = p.source_app,
       plan_slug  = 'free'
  FROM affiliate_products p
 WHERE p.system_tag_id = t.id
   AND p.source_app IS NOT NULL
   AND t.source_app IS NULL;

-- ── FunnelSwift's own two tags: the plan IS in this database, so link it for real ────────────────
-- Matched on the plan's name appearing inside the tag's name: 'FunnelSwift — Capture Free' -> the
-- 'Capture Free' plan. Restricted to system tags that belong to FunnelSwift and are still unlinked.
UPDATE tags t
   SET source_app = 'funnelswift',
       plan_id    = p.id,
       plan_slug  = p.slug
  FROM plans p
 WHERE t.is_system
   AND t.name LIKE 'FunnelSwift — %'
   AND t.name = 'FunnelSwift — ' || p.name
   AND t.plan_id IS NULL;

-- ── the two missing FunnelSwift products ────────────────────────────────────────────────────────
-- David confirmed both free plans exist because they are two different entry points, and that each
-- affiliate product should be connected to its tag. Without these rows FunnelSwift was absent from its
-- own affiliate catalogue: two tags with nothing behind them.
-- `tenant_id` is the SYSTEM tenant (00000000-0000-0000-0000-000000000001) because that is where the
-- fleet catalogue lives — measured: all five existing products belong to it, and the promotable list
-- in the affiliate UI was empty until it included the system tenant.
-- `commission_rate` comes from the plan itself, not from a number invented here.
-- Guarded by NOT EXISTS so a re-run is a no-op.
INSERT INTO affiliate_products
    (id, name, description, price, commission_rate, default_commission_rate, is_active,
     owner_name, product_type, source_app, tenant_id, system_tag_id, plan_id, slug, website_url)
SELECT gen_random_uuid(),
       'FunnelSwift ' || p.name,
       'FunnelSwift ' || p.name || ' — the ' || p.slug || ' entry point.',
       0,
       p.commission_rate,
       p.commission_rate,
       true,
       'SwiftSoftware',
       'software',
       'funnelswift',
       '00000000-0000-0000-0000-000000000001'::uuid,
       t.id,
       p.id,
       'funnelswift-' || p.slug,
       'https://app.funnelswift.net'
  FROM plans p
  JOIN tags t ON t.plan_id = p.id AND t.source_app = 'funnelswift'
 WHERE p.name IN ('Capture Free', 'Kinetic Free')
   AND NOT EXISTS (
        SELECT 1 FROM affiliate_products ap
         WHERE ap.source_app = 'funnelswift' AND ap.plan_id = p.id
   );

CREATE INDEX IF NOT EXISTS idx_tags_source_app ON tags(source_app);
CREATE INDEX IF NOT EXISTS idx_tags_plan ON tags(plan_id);
