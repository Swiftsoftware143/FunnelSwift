-- 088: retire the `target_software` + `routing_log` resources — one delivery engine, one credential
-- store (kanban t_0aaf0bc5).
--
-- WHY THIS FILE EXISTS
--   Two resources described one idea and differed in exactly one way: whether anything fires them.
--
--     resource          | dispatcher | delivery log | test | retries | console screens | rows
--     ------------------+------------+--------------+------+---------+-----------------+-----
--     webhooks          | YES        | yes          | yes  | yes     | 2 (tenant+admin)| 0
--     target_software   | NONE       | none         | none | none    | 3               | 0
--
--   MEASURED 2026-10-02 on the live database (`.` = 0 rows) and the deployed binary 6a711748:
--     * `webhooks` 0 rows, `target_software` 0 rows, `routing_log` 0 rows, `provider_keys` 0 rows,
--       across 16 tenants. This is the cheapest moment the decision will ever have.
--     * NO writer of `routing_log` exists anywhere in `src/` — grep finds the reader route
--       (`GET /api/v1/routing-logs`) and the SELECT only, so the admin console's "Recent routing log
--       (100)" table was permanently empty by construction.
--     * NO reader of `target_software.webhook_url` exists outside `routing_handler` CRUD and the
--       IncentiveSwift config GET, i.e. the URL is stored and never dispatched — while `webhooks`
--       (src/webhooks.rs, kanban t_431faa99) dispatches lead.created / tag.updated with HMAC-SHA256
--       signing, 3 retries through a SKIP LOCKED sweeper and a delivery log.
--     * The resource was registered TWICE: `GET|POST /api/v1/target-software` (gated
--       `has_dual_routing` + `max_routing_targets`, 1 caller = the admin "Lead Routing" view) and
--       `GET|POST /api/v1/integration-targets` + `PUT|DELETE /api/v1/integration-targets/:id`
--       (gated `max_integrations` with an admin bypass, callers = the tenant "Integrations" page and
--       the admin "Integrations" view). Three screens, two route families, two different plan gates,
--       one table.
--
-- THE ONE LIVE CONSUMER, AND WHERE ITS CREDENTIAL GOES
--   `incentiveswift_handler::get_incentiveswift_config` handed `target_software.api_key` to the
--   tenant's own device (FunnelSwift-Mobile `getIncentiveSwiftConfig` -> `getCampaigns`), matched by
--   `LOWER(name) LIKE '%incentiveswift%'`. That credential is NOT redundant with the webhook
--   signature, so it is preserved — moved to the app's established per-product credential store,
--   `provider_keys` (provider = 'incentiveswift'), which is what the CoreSwift connection already
--   uses (`coreswift::resolve_conn`) and which the Integration Center renders with an editor and a
--   test-connection probe. The step below seats the catalogue row so the tenant console can manage
--   the connection; `provider_keys` holds 0 rows, so no credential has to be migrated.
--
-- PLAN KEYS
--   `max_routing_targets` is RETIRED here (its only mechanism is the resource this file drops) and
--   its authored numbers are provably lost from nowhere — they are byte-identical to the surviving
--   `max_webhooks` on the same plans: kinetic-free 5 / 5, kinetic-pro -1 / -1, and agency carries
--   -1 with no `max_webhooks` row at all, where absence means "not configured = allow".
--   `max_integrations` is KEPT and RETARGETED to the Integration Center (COUNT(provider_keys)) in the
--   same change, so the numbers capture-starter 10 / kinetic-pro -1 / agency -1 keep their meaning
--   instead of pointing at a deleted route. `has_dual_routing` is untouched (it is a plan-page
--   marker on suite/agency: removing it from `plans.features` is a pricing decision, not this one).
--   `max_target_software` and `max_routing_rules` were already retired by 086.
--
-- IDEMPOTENT: DROP TABLE IF EXISTS and delete-by-key. A second application removes 0 rows and
--   re-asserts the same invariant. A fresh install replays 000001_initial.sql (which creates both
--   tables) and then this file, so neither object survives a from-zero run either.
-- NOTE: statement separators are kept out of these comments on purpose — some migration runners
--   split a file on the semicolon.

DROP TABLE IF EXISTS routing_log;
DROP TABLE IF EXISTS target_software;

-- The IncentiveSwift connection, seatable by the tenant in the Integration Center.
INSERT INTO available_providers (key, name, description, requires_base_url, requires_metadata, icon)
VALUES (
    'incentiveswift',
    'IncentiveSwift',
    'Send opted-in leads to IncentiveSwift and read your IncentiveSwift campaigns',
    false,
    '[]'::jsonb,
    'hub'
)
ON CONFLICT (key) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    requires_base_url = EXCLUDED.requires_base_url,
    requires_metadata = EXCLUDED.requires_metadata,
    icon = EXCLUDED.icon;

DO $retire$
DECLARE
    n_removed int;
    n_left    int;
BEGIN
    DELETE FROM feature_limits WHERE feature_key = 'max_routing_targets';
    GET DIAGNOSTICS n_removed = ROW_COUNT;

    -- A retire that leaves a row behind is the failure this file exists to prevent: the key would
    -- still resolve the moment a gate is called with it.
    SELECT count(*) INTO n_left FROM feature_limits WHERE feature_key = 'max_routing_targets';
    IF n_left <> 0 THEN
        RAISE EXCEPTION 'retire: max_routing_targets rows survived the migration';
    END IF;

    IF to_regclass('public.target_software') IS NOT NULL
       OR to_regclass('public.routing_log') IS NOT NULL THEN
        RAISE EXCEPTION 'retire: target_software / routing_log survived the migration';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM available_providers WHERE key = 'incentiveswift') THEN
        RAISE EXCEPTION 'seed: the incentiveswift provider row is missing';
    END IF;

    RAISE NOTICE 'retire: target_software + routing_log dropped, % max_routing_targets row(s) retired',
        n_removed;
END
$retire$;
