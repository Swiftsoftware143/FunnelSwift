-- Migration 081: connect FunnelSwift's two system tags to the free plan the install ACTUALLY has
-- (kanban t_3641f326, measured 2026-10-02).
--
-- WHY THIS FILE EXISTS
--
-- FunnelSwift's shipped code binds a plan vocabulary as if every install carried it, and the two
-- databases hold DISJOINT ones. Measured on the live `funnelswift` database vs a from-zero database
-- booted from the deployed artifact (68 migrations, ledger version 80):
--
--   slug                        live   from-zero   bound by
--   capture-free                  1        0       auth/handlers.rs register (the capture signup)
--   kinetic-free                  1        0       public_signup_handler (the kinetic signup)
--   pro / enterprise              0        1 each  tag_logic's paid test, via the slug literal
--   free (price 0, generic)       0        1       what a from-zero install actually ships
--
-- The CODE half of that defect is fixed in the same change (src/plan_resolver.rs binds the free
-- tier through `plans.price = 0` + `plans.side`, both NOT NULL, and `tag_logic` keys the paid test
-- on `plans.price > 0`). THIS file fixes the DATA half, which is the affiliate picker's view of the
-- same root:
--
--   067's second UPDATE linked each FunnelSwift system tag to its plan by matching the plan's NAME
--   inside the tag's name (`t.name = 'FunnelSwift - ' || p.name`). That only works where the
--   operator's plan names are 'Capture Free' / 'Kinetic Free' (live). A from-zero install ships
--   000001's generic names, so the match no-ops and both tags come up as
--   plan_slug='free', plan_id=NULL - 067's FIRST update had written plan_slug='free' for them,
--   from the affiliate product that points at each tag.
--
--   `src/handlers/affiliate_onboarding_handler.rs` reads exactly `COALESCE(t.source_app, p.source_app)`,
--   `t.plan_slug`, `t.plan_id`, `COALESCE(pl.name, t.plan_slug)` to answer "which plan does a
--   promotion land this customer on", so the picker told a different story on each install and the
--   fresh one had no `plan_id` at all - the id IS the reference for FunnelSwift's own plans (a
--   sibling app's plan can only be named by (source_app, plan_slug) because it lives in another
--   database, which is why 067 added all three columns).
--
-- WHAT THIS DOES, AND WHY IT IS A LIVE NO-OP
--
-- It resolves each tag's plan by the SAME property the fixed code binds - the install's free tier of
-- that product's side (`price = 0`, `side` in the product's side and the shared one) - not by a
-- name. On live the subquery resolves 'Capture Free' for the capture tag and 'Kinetic Free' for the
-- kinetic tag, which are exactly the id+slug those rows already carry, and the guard
-- (`plan_id IS DISTINCT FROM f.id OR plan_slug IS DISTINCT FROM f.slug`) therefore updates ZERO
-- rows. On a from-zero install it links both tags to the one price-0 plan that install has, which is
-- also the plan the fixed signup paths now hand out - so the picker and the signup agree.
--
-- Four statements — the product's own free tier, then the install's free tier as a guarded fallback,
-- once per tag — each guarded by the value comparison above, so a re-run is a no-op. On an install
-- whose operator has deleted every price-0 plan the scalar subquery is NULL and no statement touches
-- a row (a migration must never fail a boot over operator pricing).

UPDATE tags t
   SET plan_id    = f.id,
       plan_slug  = f.slug,
       source_app = 'funnelswift'
  FROM plans f
 WHERE t.is_system
   AND t.source_app = 'funnelswift'
   AND t.name = 'FunnelSwift — Capture Free'
   AND f.id = (SELECT p.id FROM plans p
                WHERE p.price = 0 AND p.side IN ('main', 'both')
                ORDER BY p.price ASC, p.slug ASC LIMIT 1)
   AND (t.plan_id IS DISTINCT FROM f.id OR t.plan_slug IS DISTINCT FROM f.slug);

-- Fallback arm, the exact mirror of `plan_resolver`'s second lookup: an install that carries no
-- price-0 plan on this product's side (a from-zero build: all four generic tiers are side 'main')
-- links the tag to the free tier the signup path actually hands out. The `NOT EXISTS` guard is
-- what keeps this a no-op on live, where a kinetic price-0 plan does exist.
UPDATE tags t
   SET plan_id    = f.id,
       plan_slug  = f.slug,
       source_app = 'funnelswift'
  FROM plans f
 WHERE t.is_system
   AND t.source_app = 'funnelswift'
   AND t.name = 'FunnelSwift — Capture Free'
   AND f.id = (SELECT p.id FROM plans p
                WHERE p.price = 0
                ORDER BY p.price ASC, p.slug ASC LIMIT 1)
   AND NOT EXISTS (SELECT 1 FROM plans p
                    WHERE p.price = 0 AND p.side IN ('main', 'both'))
   AND (t.plan_id IS DISTINCT FROM f.id OR t.plan_slug IS DISTINCT FROM f.slug);

UPDATE tags t
   SET plan_id    = f.id,
       plan_slug  = f.slug,
       source_app = 'funnelswift'
  FROM plans f
 WHERE t.is_system
   AND t.source_app = 'funnelswift'
   AND t.name = 'FunnelSwift — Kinetic Free'
   AND f.id = (SELECT p.id FROM plans p
                WHERE p.price = 0 AND p.side IN ('kinetic', 'both')
                ORDER BY p.price ASC, p.slug ASC LIMIT 1)
   AND (t.plan_id IS DISTINCT FROM f.id OR t.plan_slug IS DISTINCT FROM f.slug);

UPDATE tags t
   SET plan_id    = f.id,
       plan_slug  = f.slug,
       source_app = 'funnelswift'
  FROM plans f
 WHERE t.is_system
   AND t.source_app = 'funnelswift'
   AND t.name = 'FunnelSwift — Kinetic Free'
   AND f.id = (SELECT p.id FROM plans p
                WHERE p.price = 0
                ORDER BY p.price ASC, p.slug ASC LIMIT 1)
   AND NOT EXISTS (SELECT 1 FROM plans p
                    WHERE p.price = 0 AND p.side IN ('kinetic', 'both'))
   AND (t.plan_id IS DISTINCT FROM f.id OR t.plan_slug IS DISTINCT FROM f.slug);
