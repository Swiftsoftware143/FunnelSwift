-- ONE affiliate account per customer, at most. No downline. (David, 2026-09-29)
--
-- David: *"A customer can only have one affiliate account. And this isn't network marketing so they
-- don't need to pay attention to a downline. If one of that customers become an affiliate they do so
-- directly to the affiliate program in Swift."*
--
-- MEASURED BEFORE THIS — the numbers said something else entirely:
--     Agency / Scale  = -1  (unlimited)
--     Kinetic Pro     = -1  (unlimited)
--     Suite           = NULL (no row)
--     Kinetic Free    =  5
--     Capture Free    =  0   <- blocked onboarding entirely
--     Capture Starter =  0   <- same
-- `max_affiliates` is enforced as a hard gate by create_affiliate
-- (features::enforce_feature_limit), so a Capture-Free customer was refused an affiliate outright,
-- while an unlimited plan allowed any number. Neither matches "one per customer".
--
-- Setting the limit to 1 on EVERY plan means the existing gate now expresses the real rule for
-- everyone: one affiliate account, on any plan, including free. That also removes the contradiction
-- where signing up as an affiliate worked but an admin creating one was refused — same action, and
-- now the same answer on every plan.
--
-- The unique index makes it structural rather than merely gated: without it, two different code paths
-- (self-serve signup, admin create) could disagree again, and the admin path only checked for a
-- duplicate EMAIL, not a duplicate ACCOUNT. `affiliates` is empty today, so this cannot collide.
--
-- NOT touched: the commission rate per plan. Nobody is paid on the free plans because the free plans
-- are the entry point, and the payout rule itself is enforced in the money path separately.

-- 1. one per plan, everywhere, including plans that had no row at all
INSERT INTO feature_limits (id, plan_id, feature_key, limit_value)
SELECT gen_random_uuid(), p.id, 'max_affiliates', 1
  FROM plans p
 WHERE NOT EXISTS (
        SELECT 1 FROM feature_limits f
         WHERE f.plan_id = p.id AND f.feature_key = 'max_affiliates'
 );

UPDATE feature_limits
   SET limit_value = 1
 WHERE feature_key = 'max_affiliates'
   AND limit_value <> 1;

-- 2. make it structural: one affiliate row per customer account.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_affiliates_tenant
    ON affiliates (tenant_id);
