-- Automatic lead sync into CoreSwift: per-workspace switch + a visible outcome per lead.
--
-- WHY (kanban t_934b3a07, David 2026-11-28): "All the CEOs are using n8n to get their leads into
-- CoreSwift. That should not be necessary." The direct connector already existed but had no switch
-- and no record of what happened to a given lead, so a workspace could not tell whether its capture
-- had reached its CRM without reading the app log — which is why every CEO wired an n8n hop instead.
--
-- 1. `tenant_settings` key `coreswift_sync` is the switch (jsonb):
--      {"enabled": true, "list_id": "<hub list uuid>", "tags": ["tag", "..."]}
--    ABSENT means OFF: no workspace starts shipping its leads anywhere by surprise. The backfill at
--    the bottom opts in exactly the workspaces that have ALREADY opted in by storing an active
--    CoreSwift key, so nobody's working integration stops on this deploy.
--
-- 2. The outcome of each lead's push is recorded on the lead itself, so a failure is visible on the
--    row the customer can open rather than only in a log line:
--      coreswift_sync_status  pushed | failed | skipped   (NULL = sync was off, never attempted)
--      coreswift_sync_detail  the hub's own words when it refused
--      coreswift_synced_at    when it last succeeded
--
-- 3. The durable retry queue (migration 146) gains `lead_id`, so a retried push can correct the same
--    lead's recorded outcome instead of leaving a permanent "failed" behind a successful retry.

ALTER TABLE leads ADD COLUMN IF NOT EXISTS coreswift_sync_status TEXT;
ALTER TABLE leads ADD COLUMN IF NOT EXISTS coreswift_sync_detail TEXT;
ALTER TABLE leads ADD COLUMN IF NOT EXISTS coreswift_synced_at TIMESTAMPTZ;

-- The console's "which captures did not reach my CRM" read.
CREATE INDEX IF NOT EXISTS idx_leads_coreswift_sync_failed
    ON leads (tenant_id, created_at DESC)
    WHERE coreswift_sync_status = 'failed';

ALTER TABLE coreswift_push_queue ADD COLUMN IF NOT EXISTS lead_id UUID;

-- Opt in every workspace that already holds a live CoreSwift key: that key WAS the opt-in (there was
-- no other way to turn the push on), and the push has been running for these workspaces since the
-- BYOK client shipped. Writing the row keeps their behaviour identical on deploy day while leaving
-- every other workspace OFF.
INSERT INTO tenant_settings (id, tenant_id, key, value)
SELECT gen_random_uuid(), DISTINCT_KEYS.tenant_id, 'coreswift_sync', '{"enabled": true}'::jsonb
  FROM (SELECT DISTINCT tenant_id FROM provider_keys
         WHERE provider = 'coreswift' AND is_active = TRUE AND COALESCE(api_key, '') <> '') AS DISTINCT_KEYS
ON CONFLICT (tenant_id, key) DO NOTHING;
