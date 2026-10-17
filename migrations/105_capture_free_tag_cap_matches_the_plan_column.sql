-- 105: the capture-free tag cap — the panel's number and the gate's number are now one number.
--
-- MEASURED 2026-10-17. `plans.max_tags` for `capture-free` is 2 (that row was last edited
-- 2026-09-20) and every customer-facing surface renders THAT column: the dashboard's "Your Plan"
-- strip ("Tags 0 / 2"), the pricing page and the admin plan editor. But a `feature_limits` row
-- seeded on 2026-06-27 still carried 5, and `features::resolved_limit` reads the per-plan row
-- BEFORE the plan column — so the tag gate enforced 5 while the panel promised 2. Proven live: a
-- capture-free probe tenant created FOUR tags (201 "Tag created" each) against a panel showing
-- "Tags 0 / 2", i.e. the allowance a customer is told and the allowance the gate holds them to
-- disagreed for six weeks.
--
-- This recomputes the stale override to the plan's own column, so the number the customer reads
-- and the number the gate refuses on are the same. Idempotent: once they agree the WHERE matches
-- no row, and a later edit to either side (the admin panel's own override lever still works:
-- it writes a value that then differs and wins) is untouched by re-running this file.
UPDATE feature_limits f
   SET limit_value = p.max_tags
  FROM plans p
 WHERE f.plan_id = p.id
   AND p.slug = 'capture-free'
   AND f.feature_key = 'max_tags'
   AND p.max_tags IS NOT NULL
   AND f.limit_value IS DISTINCT FROM p.max_tags;
