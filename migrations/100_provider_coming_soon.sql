-- 100_provider_coming_soon.sql
-- FunnelSwift: six rows in the Integration Center catalogue are offered as connectable and ACCEPT a
-- stored credential, but NO code path in this app can ever spend them. A customer pastes a real
-- OpenAI / DeepSeek / Nexweave / SAM.gov / USASpending / Telnyx key, POST /api/v1/provider-keys
-- answers 200 and stores the row, and nothing ever reads it. Same defect class the ADASwift lane
-- closed with migration 010 and the WorkflowSwift lanes with 083-087. Kanban t_c10c05b6.
--
-- WHERE THE SIX COME FROM: the baseline seed migration 000007_provider_keys.sql inserted eight
-- providers (mailgun, sendgrid, deepseek, openai, nexweave, sam_gov, usaspending, telnyx) as a
-- generic starting catalogue. Migration 049_integration_center.sql then added THIS app's real
-- integration set (coreswift, smtp, sendiio, mailgun, sendgrid, ocr, kinetic, affiliate_payout,
-- per the fleet standard /opt/swift/docs/integration-center-standard-2026-09-20.md). The five AI /
-- federal-discovery rows plus telnyx were never implemented here and are not in FunnelSwift's
-- declared integration list.
--
-- MEASURED 2026-10-11 (off-peak, live app + live database):
--   * SELECT key FROM available_providers -> 15 rows, none carried a coming_soon flag (the column
--     did not exist); all six below were served connectable and rendered with a key box.
--   * A repo-wide case-insensitive grep over src/ for each of the six returns ZERO hits (telnyx 0,
--     openai 0 except one doc comment in src/security/provider_key_crypto.rs that merely names
--     OpenAI as an example of a customer-supplied credential, deepseek 0, nexweave 0, sam_gov 0,
--     usaspending 0). The ONLY providers that read provider_keys are
--       src/coreswift.rs + src/handlers/coreswift_integration_handler.rs   (provider 'coreswift')
--       src/handlers/incentiveswift_handler.rs                            (provider 'incentiveswift')
--     and the email providers are spent through tenant_settings.email_config, not provider_keys.
--   * There is no env-var reader either: src/config.rs reads DATABASE_URL, JWT_SECRET,
--     INTERNAL_SYNC_KEY, HOST/PORT and the mail settings - no AI / SAM.gov / USASpending / Telnyx
--     key anywhere - and nothing resolves a provider by catalogue category, so a stored key cannot
--     be reached indirectly.
--
-- WHY coming_soon AND NOT is_active = false: David's standing rule is LABEL, DON'T HIDE. The rows
-- stay visible in both consoles and gain coming_soon = true. The two code halves that read the flag
-- ship in the same commit:
--   * handlers/provider_keys_handler.rs::list_available_providers now serves coming_soon
--   * handlers/provider_keys_handler.rs::upsert_provider_key now refuses a flagged row with 400
--     "<name> is Coming Soon and cannot be connected yet - nothing in FunnelSwift reads its key.",
--     and refuses a provider that is not in the catalogue at all, so the free-text form can no
--     longer store a credential for a name this app does not know.
--
-- REVERSAL (one line each): a row returns to connectable with
--   UPDATE available_providers SET coming_soon = false WHERE key = 'openai';
-- It then needs the missing half named - a real dispatch arm for that provider plus whatever
-- credential reader it needs.
--
-- ADDITIVE + IDEMPOTENT: ADD COLUMN IF NOT EXISTS is safe on a database that already has the column
-- (a fresh install gets it from this file) and the UPDATE is a no-op once applied. provider_keys
-- holds ZERO rows on production, so no customer stored credential is affected by the refusal.
-- No BEGIN/COMMIT and no semicolons inside comments - the runner wraps the file.

ALTER TABLE available_providers
    ADD COLUMN IF NOT EXISTS coming_soon boolean NOT NULL DEFAULT false;

UPDATE available_providers
   SET coming_soon = true
 WHERE key IN ('openai', 'deepseek', 'nexweave', 'sam_gov', 'usaspending', 'telnyx');
