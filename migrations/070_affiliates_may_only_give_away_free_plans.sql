-- AFFILIATE PRODUCTS MAY ONLY EVER HAND OUT A FREE PLAN (David, 2026-09-29).
--
-- David: *"Funnel Swift will only deal with the free plans from each software so we don't risk an
-- affiliate giving away a software fully upgraded for free."*
--
-- MEASURED BEFORE THIS: all 7 affiliate products happened to point at free plans, but **nothing
-- enforced it**. `affiliate_products.plan_id` and `tags.plan_id` are plain foreign keys to `plans`,
-- so any code path, admin screen, or hand-edit could point a product at Suite ($29) or
-- Agency / Scale ($79) — and an affiliate promoting that product would hand the customer a paid
-- product for nothing. The whole point of the affiliate programme is that the customer arrives on a
-- free plan and UPGRADES IN APP, paying for it themselves.
--
-- The guard is a TRIGGER rather than an application check, because there are several write paths
-- (admin console, API, cross-app sync, migrations) and a rule that only lives in one of them is a
-- rule that will be bypassed. It is deliberately narrow so it cannot break legitimate work:
--   * a product with NO plan link is allowed (the 5 sibling products link by tag, not by plan),
--   * a product linked to a plan with price 0 (or NULL, treated as free) is allowed,
--   * a SIBLING app's plan is judged by `tags.plan_slug`, since that plan lives in another database
--     and has no row here to price-check. 'free' is the only accepted slug for a sibling tag.
-- Everything else raises, naming what it refused.

CREATE OR REPLACE FUNCTION enforce_affiliate_products_are_free()
RETURNS TRIGGER AS $$
DECLARE
    plan_price numeric;
    tag_app    varchar;
    tag_slug   varchar;
BEGIN
    -- 1. A plan linked directly (FunnelSwift's own plans).
    IF NEW.plan_id IS NOT NULL THEN
        SELECT price INTO plan_price FROM plans WHERE id = NEW.plan_id;
        IF plan_price IS NOT NULL AND plan_price <> 0 THEN
            RAISE EXCEPTION
                'affiliate product "%" may not be linked to a PAID plan (price %). Affiliates may only '
                'hand out free plans; the customer upgrades in app and pays for it themselves.',
                COALESCE(NEW.name, NEW.id::text), plan_price;
        END IF;
    END IF;

    -- 2. A sibling app's plan, referenced through the tag.
    IF NEW.system_tag_id IS NOT NULL THEN
        SELECT source_app, plan_slug INTO tag_app, tag_slug
          FROM tags WHERE id = NEW.system_tag_id;
        IF tag_app IS NOT NULL
           AND tag_app <> 'funnelswift'
           AND tag_slug IS NOT NULL
           AND lower(tag_slug) <> 'free' THEN
            RAISE EXCEPTION
                'affiliate product "%" is tagged for the % plan "%"; affiliates may only hand out the '
                'free plan of each software.',
                COALESCE(NEW.name, NEW.id::text), tag_app, tag_slug;
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_affiliate_products_free_only ON affiliate_products;
CREATE TRIGGER trg_affiliate_products_free_only
    BEFORE INSERT OR UPDATE OF plan_id, system_tag_id, name
    ON affiliate_products
    FOR EACH ROW
    EXECUTE FUNCTION enforce_affiliate_products_are_free();

-- The same rule for the TAG side, so a tag cannot be re-pointed at a paid plan either.
CREATE OR REPLACE FUNCTION enforce_tags_point_at_free_plans()
RETURNS TRIGGER AS $$
DECLARE
    plan_price numeric;
BEGIN
    IF NEW.plan_id IS NOT NULL THEN
        SELECT price INTO plan_price FROM plans WHERE id = NEW.plan_id;
        IF plan_price IS NOT NULL AND plan_price <> 0 THEN
            RAISE EXCEPTION
                'tag "%" may not point at a PAID plan (price %). Affiliate tags only ever name a free '
                'plan, so an affiliate cannot give a paid product away.',
                NEW.name, plan_price;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_tags_free_plan_only ON tags;
CREATE TRIGGER trg_tags_free_plan_only
    BEFORE INSERT OR UPDATE OF plan_id ON tags
    FOR EACH ROW
    EXECUTE FUNCTION enforce_tags_point_at_free_plans();
