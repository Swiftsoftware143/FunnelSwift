-- AF-5 for FunnelSwift (kanban t_35acff73): the TOP TIER grants EVERY key the feature registry
-- (src/feature_registry.rs) defines. David, 2026-09-23: "The top tier plan gets everything."
--
-- MEASURED BEFORE THIS (live, 2026-10-02):
--   * top tier = `agency` (Agency / Scale, $79/mo, $790/yr) — `plans` has no `sort_order` and no
--     `is_active` column, so "top" is the highest price, then name (the registry's own rule).
--   * the registry defines 16 numeric limits and 11 booleans; the top tier carried NO
--     `feature_limits` row at all for 7 of the limit keys (max_cards, max_forms, max_custom_domains,
--     max_qr_codes, max_ocr_scans, max_action_buttons, max_team_members), and no `features` jsonb
--     key for 4 of the booleans (api_access, webhooks, import_export, analytics — the chips were
--     only on the `has_*` columns, which the gate falls back to but the admin panel does not show).
--   * the ABSENCE rule is why the missing rows matter: a limit with no row and a NULL column is
--     "not configured", which `enforce_feature_limit` treats as ALLOW — so those keys were INERT
--     (they enforced nothing) rather than granted. The grant makes the state explicit and shows up
--     in the panel.
--
-- DELIBERATELY DERIVED FROM THE DATA as well as the registry: the UNION below also picks up every
-- key configured on ANY plan, so a key added to another plan later is still covered by "the top
-- tier has it". Idempotent — re-running changes nothing, and the admin panel can re-press the same
-- grant with POST /api/v1/admin/plans/grant-top-tier (gap-filling only).

DO $$
DECLARE
    top_plan uuid;
    top_name text;
BEGIN
    SELECT id, name INTO top_plan, top_name
      FROM plans
     ORDER BY COALESCE(price, 0) DESC, name
     LIMIT 1;

    IF top_plan IS NULL THEN
        RAISE NOTICE 'AF-5: no plans found; nothing to grant';
        RETURN;
    END IF;

    -- 1. Every numeric limit the registry defines, at this app's OWN unlimited convention (-1,
    --    which Kinetic Pro already used on 14 keys), for any key the top tier does not configure.
    --    Existing rows are LEFT ALONE: a cap the owner set — e.g. max_affiliates = 1, the
    --    one-affiliate-per-customer rule from migration 069 — is never silently raised.
    INSERT INTO feature_limits (id, plan_id, feature_key, limit_value)
    SELECT gen_random_uuid(), top_plan, keys.feature_key, -1
      FROM (
            SELECT unnest(ARRAY[
                'max_cards', 'max_leads', 'max_tags', 'max_forms', 'max_custom_domains',
                'max_qr_codes', 'max_ocr_scans', 'max_action_buttons', 'max_webhooks',
                'max_api_keys', 'max_portfolios', 'max_tag_groups', 'max_routing_targets',
                'max_integrations', 'max_team_members'
            ]) AS feature_key
            UNION
            SELECT DISTINCT feature_key FROM feature_limits
           ) keys
     WHERE NOT EXISTS (
            SELECT 1 FROM feature_limits f
             WHERE f.plan_id = top_plan AND f.feature_key = keys.feature_key
     );

    -- 2. Every boolean flag the registry defines, ON, in BOTH stores the app reads: the
    --    `plans.features` jsonb key (the gate's first choice, and what the panel's feature editor
    --    shows the owner) AND the legacy `has_*` column (which the card renderer reads with raw
    --    SQL). Writing only one of the two is how "the panel says no / the gate says yes" starts.
    UPDATE plans
       SET features = COALESCE(features, '{}'::jsonb) || jsonb_build_object(
               'api_access', true,
               'webhooks', true,
               'import_export', true,
               'analytics', true,
               'dual_routing', true,
               'mini_funnels', true,
               'card_gating', true,
               'white_label', true,
               'remove_branding', true,
               'multi_tenant', true,
               'premium_themes', true
           ),
           has_api            = true,
           has_webhooks       = true,
           has_import_export  = true,
           has_analytics      = true,
           has_dual_routing   = true,
           has_mini_funnels   = true,
           has_card_gating    = true,
           has_white_label    = true,
           has_remove_branding= true,
           has_multi_tenant   = true,
           updated_at         = NOW()
     WHERE id = top_plan;

    RAISE NOTICE 'AF-5: granted every registry feature to the top tier % (%)', top_name, top_plan;
END $$;
