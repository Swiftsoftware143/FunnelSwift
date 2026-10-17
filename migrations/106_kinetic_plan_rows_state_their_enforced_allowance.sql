-- 106: a plan's own row now states the allowance its gate enforces (Kinetic + Capture Starter).
--
-- MEASURED 2026-10-17 against the live plan matrix. The customer-facing "Your Plan" strip and the
-- pricing page render `plans.max_*`, while every write gate resolves the `feature_limits` row (per
-- plan) BEFORE the plan column. Where the two disagreed, the panel told the customer something the
-- app then contradicted — in both directions:
--
--   kinetic-free     max_leads  column 0      row 1000   -> panel "not on this plan", gate allowed 1000
--   kinetic-free     max_tags   column 0      row 20     -> panel "not on this plan", gate allowed 20
--   kinetic-pro      max_leads  column 0      row -1     -> panel "not on this plan", gate unlimited
--   kinetic-pro      max_tags   column 0      row -1     -> panel "not on this plan", gate unlimited
--   capture-starter  max_leads  column NULL   row 100    -> panel "unlimited",     gate stopped at 100
--   capture-starter  max_tags   column NULL   row 5      -> panel "unlimited",     gate stopped at 5
--
-- One tenant is live on kinetic-free today. The override is what the gate reads, so this writes the
-- OVERRIDE's number into the plan's own column: enforcement does not move by a single unit (the
-- override is untouched and still wins), and what the customer is TOLD becomes what the app does.
-- Two statements, one per key, so a plan carrying both overrides cannot have one value land in the
-- other's column. Idempotent: after the run the two sides agree and each WHERE matches nothing; a
-- later deliberate edit to either side is left alone.
--
-- NOT touched here: `agency` (column 10 domains / 1 seat, override -1). Aligning that one would
-- publish "unlimited" on a paid tier — a product decision for David, recorded on the board instead.
UPDATE plans p
   SET max_leads = f.limit_value, updated_at = now()
  FROM feature_limits f
 WHERE f.plan_id = p.id
   AND p.slug IN ('kinetic-free', 'kinetic-pro', 'capture-starter')
   AND f.feature_key = 'max_leads'
   AND (to_jsonb(p) ->> 'max_leads') IS DISTINCT FROM f.limit_value::text;

UPDATE plans p
   SET max_tags = f.limit_value, updated_at = now()
  FROM feature_limits f
 WHERE f.plan_id = p.id
   AND p.slug IN ('kinetic-free', 'kinetic-pro', 'capture-starter')
   AND f.feature_key = 'max_tags'
   AND (to_jsonb(p) ->> 'max_tags') IS DISTINCT FROM f.limit_value::text;
