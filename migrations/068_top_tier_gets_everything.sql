-- THE TOP TIER GETS EVERYTHING (David, 2026-09-29: "assigned those 14 features ... to the highest
-- tier plan ... Everything is editable I will straighten that out when I go in and I assigned the
-- plans.").
--
-- MEASURED BEFORE THIS: FunnelSwift's highest-priced plan, **Agency / Scale** ($79/mo, $790/yr), had
-- **ZERO rows in `feature_limits`** — no limits defined at all — and `has_multi_tenant = false` while
-- the CHEAPER Kinetic Pro ($9) carried `-1` (unlimited) on 14 different keys. So the most expensive
-- plan was the emptiest, and a customer upgrading to it would have lost limits they already had.
--
-- `-1` is this app's existing convention for unlimited (Kinetic Pro uses it on every key), so
-- "everything" means every key that exists anywhere, at `-1`, plus every boolean toggle on.
--
-- Deliberately derived FROM THE DATA rather than a hardcoded list: if a feature key is added to any
-- other plan later, this still means "the top tier has it". Idempotent — re-running changes nothing.

DO $$
DECLARE top_plan uuid;
BEGIN
    SELECT id INTO top_plan
      FROM plans
     ORDER BY COALESCE(price, 0) DESC, name
     LIMIT 1;

    IF top_plan IS NULL THEN
        RAISE NOTICE 'no plans found; nothing to grant';
        RETURN;
    END IF;

    -- Every limit key that exists on ANY plan, at unlimited, on the top tier.
    INSERT INTO feature_limits (id, plan_id, feature_key, limit_value)
    SELECT gen_random_uuid(), top_plan, k.feature_key, -1
      FROM (SELECT DISTINCT feature_key FROM feature_limits) k
     WHERE NOT EXISTS (
            SELECT 1 FROM feature_limits f
             WHERE f.plan_id = top_plan AND f.feature_key = k.feature_key
     );

    -- Anything the top tier already had, raised to unlimited.
    UPDATE feature_limits
       SET limit_value = -1
     WHERE plan_id = top_plan
       AND limit_value <> -1;

    -- Every capability toggle on. `has_multi_tenant` was false on EVERY plan, so this is the first
    -- plan to carry it — which is the point of the rule.
    UPDATE plans
       SET has_api            = true,
           has_webhooks       = true,
           has_white_label    = true,
           has_multi_tenant   = true,
           has_analytics      = true,
           has_import_export  = true,
           has_remove_branding= true,
           has_dual_routing   = true,
           has_mini_funnels   = true,
           has_card_gating    = true,
           updated_at         = NOW()
     WHERE id = top_plan;
END $$;
