-- David's commission model (2026-09-29, kanban FS-7).
--
-- Three things he asked for that the schema could not express:
--
--   1. "a certain commission for each product or service that's added"
--      Already possible: affiliate_products.default_commission_rate (the field both admin consoles
--      edit) with a legacy `commission_rate` column beside it. Nothing here.
--
--   2. "do an override and do an overall by selecting which products are the same"
--      NOT possible before this migration: there was no way to say that several products share one
--      rate. affiliate_product_groups is that concept, and affiliate_products.group_id is the
--      membership. A group rate applies to every member.
--
--   3. "override a particular affiliate a higher commission on top of whatever they're getting"
--      affiliates.commission_rate already held a number, but nothing distinguished "this affiliate's
--      standing rate" from "this affiliate was given something better" — one column, two meanings, so
--      a raised rate was indistinguishable from a base rate after the fact. override_commission_rate
--      is that distinction and it takes precedence over every product/group/plan rate.
--
-- Additive and reversible by construction: two nullable columns and one new table. Nothing existing
-- is dropped, renamed, narrowed or given a NOT NULL. With no group rows and no overrides set, the
-- resolution order falls through to exactly the columns that were already there.

-- ── product groups: products that share one commission rate ─────────────────────────────────────
CREATE TABLE IF NOT EXISTS affiliate_product_groups (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       uuid,
    name            varchar(255) NOT NULL,
    commission_rate numeric(5,2),
    description     text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_affiliate_product_groups_tenant
    ON affiliate_product_groups(tenant_id);

-- A product belongs to at most one group. ON DELETE SET NULL, not CASCADE: deleting a group must
-- never delete the products inside it — they simply fall back to their own rate.
ALTER TABLE affiliate_products
    ADD COLUMN IF NOT EXISTS group_id uuid REFERENCES affiliate_product_groups(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_affiliate_products_group ON affiliate_products(group_id);

-- ── per-affiliate override ──────────────────────────────────────────────────────────────────────
-- Deliberately separate from commission_rate so "they have a standing rate of 10%" and "they were
-- given 25% as an override" are both recoverable. NULL = no override, use the normal chain.
ALTER TABLE affiliates
    ADD COLUMN IF NOT EXISTS override_commission_rate numeric(5,2);

-- Why the override exists, in the admin's own words. Auditable, and prevents the "why is this
-- affiliate on 25%?" conversation a year from now.
ALTER TABLE affiliates
    ADD COLUMN IF NOT EXISTS override_note text;
