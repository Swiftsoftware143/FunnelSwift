-- t_850781e2 — give `feature_limits` the two guards it never had: a plan FK and per-(plan, key) uniqueness.
--
-- WHY (measured 2026-10-02 on the live funnelswift DB, census in
-- /opt/swift/audits/t_850781e2/01-orphan-census.txt):
--   * `feature_limits` was created (migrations/022_create_missing_tables.sql:26-32) with NO foreign key
--     on `plan_id` and NO unique constraint on `(plan_id, feature_key)` — only a primary key. Every
--     migration since (068, 079, 086, 088, 090) has been writing into that shape.
--   * MEASURED before this file: 43 rows total = 34 on live plans + 9 ORPHAN rows whose `plan_id` is
--     not a `plans.id` at all, across 3 deleted plans:
--         255d0c79-02a7-4945-8f9b-c6c1d8e58b79 | max_affiliates=1  max_leads=100  max_tags=5
--         6bb56fae-1fa2-475c-8b9b-43f7cba2f844 | max_affiliates=1  max_leads=100  max_tags=5
--         ef2d8708-7707-4d89-85aa-c447f5e58e43 | max_affiliates=1  max_leads=100  max_tags=5
--     The card that surfaced them (t_090f3e00) reported 19 rows across 4 deleted plans; migration 086
--     DELETEd the 10 rows that belonged to the 9 retired keys, leaving exactly these 9.
--   * The app has a route that DELETEs a plan (plan_handler::delete_plan_admin ->
--     `DELETE FROM plans WHERE id = $1`, src/handlers/plan_handler.rs:423) and it does not touch
--     `feature_limits`. So the orphans are not a one-off accident: every plan delete mints more of
--     them. That is the defect the FK closes.
--   * 0 duplicate `(plan_id, feature_key)` groups existed when this ran, so the UNIQUE is satisfiable
--     as a data no-op.
--
-- REACHABILITY — decided per reader, not assumed (census §C):
--   * `features::resolved_limit` (src/features.rs:186-196) is the ONLY path a gate reads a row
--     through, and it joins `tenant_plan_subscriptions tps ON tps.plan_id = fl.plan_id`.
--     `tps.plan_id` is FK'd to `plans(id)`, so a subscription can never name a deleted plan
--     (measured: 0 dangling subscriptions). The 9 orphans are therefore UNREACHABLE by every gate.
--   * the panel matrix (plan_handler::admin_plan_registry) looks the row up BY a live `plans` id it
--     got from `plan_rows` (SELECT to_jsonb(p) FROM plans p), so it never matches an orphan either.
--   * the only write door (plan_handler::resolve_plan_id) is `SELECT id FROM plans WHERE id/slug = $1`,
--     so no route can re-seat a row for a plan that does not exist.
--   Unreachable today — but a live loaded gun the moment a plan id is reused (they are UUIDs, so a
--   human-initiated reuse, not a natural collision), which is why they are DELETEd rather than kept.
--
-- DECISION:
--   1. DELETE the 9 orphan rows. Nothing could honour them (no gate, no panel cell, no writer) and
--      their only effect is to make a plan-less row visible to `feature_limit_rows`.
--   2. ADD `feature_limits_plan_id_fkey`  FOREIGN KEY (plan_id) REFERENCES plans(id) ON DELETE CASCADE.
--      The CASCADE is the shape the fleet already uses for this exact table (missedcallrespondr
--      migrations/000011_schema_fix.sql:27-33) and it is what FunnelSwift itself uses for every other
--      plan-owned child (000001_initial.sql:82; affiliate_products 026 -> SET NULL, tags 067 -> SET
--      NULL). `plan_id` stays NULLABLE: the column always was, and tightening it is a separate change.
--      Recurrence is the point — after this file a plan delete can no longer leave a limit row behind.
--   3. ADD `feature_limits_plan_id_feature_key_key` UNIQUE (plan_id, feature_key) — the missing guard
--      that makes `feature_registry::write_limit` atomic (it becomes `INSERT … ON CONFLICT
--      (plan_id, feature_key) DO UPDATE`, shipped in the same commit). Before it, a concurrent panel
--      write could seat a duplicate row and `resolved_limit` would pick one by `tps.start_date`
--      ordering instead of by the operator's intent.
--
-- SCOPE — deliberately NOT in this file:
--   * the 34 rows on LIVE plans are untouched byte-for-byte (fingerprints fp_registry/fp_plans in
--     /opt/swift/audits/t_850781e2/ are identical before/after);
--   * `plans` is untouched;
--   * `plan_id` is not made NOT NULL, `created_at`'s default is not changed.
--
-- Idempotent: the DELETE matches nothing on a second run, and both constraints are added under an
-- IF-NOT-EXISTS guard so a host that already has them (or a fresh install re-running the file) is a
-- no-op rather than an error.
--
-- Reversal (from the archive at /opt/swift/audits/t_850781e2/02-orphan-rows-archived.csv):
--   ALTER TABLE public.feature_limits DROP CONSTRAINT IF EXISTS feature_limits_plan_id_fkey;
--   ALTER TABLE public.feature_limits DROP CONSTRAINT IF EXISTS feature_limits_plan_id_feature_key_key;
--   -- then re-INSERT the archived rows if they are ever wanted again.

-- 1. the orphans go first: the FK below would refuse them with 23503.
DELETE FROM public.feature_limits fl
 WHERE fl.plan_id IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM public.plans p WHERE p.id = fl.plan_id);

-- 2. a plan delete now cleans up its limits instead of orphaning them.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
     WHERE conname = 'feature_limits_plan_id_fkey'
       AND conrelid = 'public.feature_limits'::regclass
  ) THEN
    ALTER TABLE public.feature_limits
      ADD CONSTRAINT feature_limits_plan_id_fkey
      FOREIGN KEY (plan_id) REFERENCES public.plans(id) ON DELETE CASCADE;
  END IF;
END $$;

-- 3. one row per (plan, key) — the guard `write_limit`'s ON CONFLICT target now names.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
     WHERE conname = 'feature_limits_plan_id_feature_key_key'
       AND conrelid = 'public.feature_limits'::regclass
  ) THEN
    ALTER TABLE public.feature_limits
      ADD CONSTRAINT feature_limits_plan_id_feature_key_key
      UNIQUE (plan_id, feature_key);
  END IF;
END $$;
