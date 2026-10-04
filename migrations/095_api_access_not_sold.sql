-- FunnelSwift — stop selling "API access" on any plan.
--
-- David, 2026-10-04: *"no API access to funnel Swift"*.
--
-- MEASURED before this ran: `plans.has_api` was TRUE on three of six plans (agency, capture-starter,
-- suite) and `features->>'api_access'` was true on two of them. The feature-registry ENTRY had already
-- been retired, with the comment that the plan VALUES were "a pricing call, not this crate's" — the
-- pricing call is now made: no plan sells API access.
--
-- Reproducible rather than a one-off DB edit, so a fresh install cannot re-sell it.
UPDATE plans
   SET has_api  = false,
       features = features - 'api_access'
 WHERE has_api = true OR features ? 'api_access';
