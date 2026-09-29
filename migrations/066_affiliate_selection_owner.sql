-- Affiliate onboarding: an affiliate choosing which products to promote (2026-09-29).
--
-- David's contract: *"an affiliate tab ... so they can pick the product they want to promote."*
--
-- `affiliate_selections` already existed with the right intent — id, tenant_id, product_id, code,
-- is_active, created_at — but **nothing in the codebase ever read or wrote it** (0 code references,
-- 0 rows): a phantom table, the same failure mode as IncentiveSwift's knowledge_base. It also had no
-- way to say WHICH affiliate selected a product, only which tenant, so two affiliates in one tenant
-- could not promote different products.
--
-- Additive: one nullable column and one index. Existing rows (there are none) would keep working with
-- affiliate_id NULL, which this app treats as "the tenant's own default selection".

ALTER TABLE affiliate_selections
    ADD COLUMN IF NOT EXISTS affiliate_id varchar(50);

-- One row per affiliate per product. Partial-unique (IS NOT NULL) so any legacy tenant-level rows
-- with a NULL affiliate_id cannot collide with each other.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_affiliate_selections_affiliate_product
    ON affiliate_selections(affiliate_id, product_id)
    WHERE affiliate_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_affiliate_selections_tenant
    ON affiliate_selections(tenant_id);
