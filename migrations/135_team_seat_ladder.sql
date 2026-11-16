-- kanban t_8e9d41d7 — THE FLEET TEAM-SEAT LADDER (pricing decision, 2026-11-16)
--
-- WHAT WAS WRONG
--   Every plan on this catalogue carried `max_team_members = 1` (Capture Free, Kinetic Free,
--   Capture Starter, Kinetic Pro, Suite, Agency / Scale at 79/mo). The team pane, the invite gate
--   and the refusal wording all worked, and the console deliberately hid the "Upgrade to add a
--   teammate" link because NO plan could deliver a second seat. So the seat upsell — one of the 20%
--   in-app levers — could not be bought at any price, and the top-tier customer at 1/1 was told
--   "no plan on this installation includes more".
--
-- THE NUMBERS (seats follow PRICE, not product line: the same money buys the same team size)
--   capture-free     $0   1 seat  (the owner — every free plan is single-seat)
--   kinetic-free     $0   1
--   capture-starter  $9   3
--   kinetic-pro      $9   3
--   suite            $29  10
--   agency           $79  25
--   No plan is `-1` (unlimited) on seats, so a refusal at the top tier still names a real next step.
--   Rationale + the fleet-wide policy: docs/fleet-team-seat-policy-2026-11-16.md
--
-- WHY THE STALE `feature_limits` ROW IS DELETED
--   `features::resolved_limit` reads tenant override > `feature_limits` row > `plans` column. An
--   admin-entitlement write had left `agency -> max_team_members = 1` in `feature_limits`, which
--   OUTRANKS the column: setting the column alone would have left Agency resolving to 1 and the
--   dead end intact. Only a stale value (<= 1, i.e. the old flat default) is removed — a number an
--   operator deliberately set above the default always survives, and any later write through
--   PUT /api/v1/admin/plans/entitlement re-adds the override and wins again.
--
-- REVERSAL (one batch, if the pricing decision changes)
--   UPDATE plans SET max_team_members = 1
--    WHERE slug IN ('capture-free','kinetic-free','capture-starter','kinetic-pro','suite','agency');
--   DELETE FROM feature_limits WHERE feature_key = 'max_team_members';

UPDATE plans
   SET max_team_members = CASE slug
           WHEN 'capture-free'    THEN 1
           WHEN 'kinetic-free'    THEN 1
           WHEN 'capture-starter' THEN 3
           WHEN 'kinetic-pro'     THEN 3
           WHEN 'suite'           THEN 10
           WHEN 'agency'          THEN 25
       END,
       updated_at = NOW()
 WHERE slug IN ('capture-free', 'kinetic-free', 'capture-starter',
                'kinetic-pro', 'suite', 'agency');

DELETE FROM feature_limits fl
 USING plans p
 WHERE fl.plan_id = p.id
   AND fl.feature_key = 'max_team_members'
   AND fl.limit_value <= 1;
