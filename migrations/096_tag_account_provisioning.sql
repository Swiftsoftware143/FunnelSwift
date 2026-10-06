-- Migration 096: the tag → free-account mapping becomes an EXECUTED mapping
-- (kanban t_847f9d63, design: /opt/swift/docs/tag-to-free-account-design-2026-10-06.md §3.2).
--
-- WHY THIS FILE EXISTS
--
-- `tags.source_app` + `tags.plan_slug` already carry the tag → (app, free plan) mapping on the seven
-- `<App> — Free` system tags (migrations 067/081), and the affiliate picker already renders it. What
-- was missing is the EXECUTOR: nothing read those two columns on the tag-application path, so tagging
-- a lead with "CoreSwift — Free" stored a string and stopped. This migration adds the two pieces the
-- caller needs:
--
--   1. `tags.provisions_account` — the per-tag switch that says "applying this tag must mint a free
--      account in the app it names". Shipped TRUE for the seven system free tags (the mapping exists
--      and the intent is David's: 2026-10-05, "tagging must create a free account in that app"). A
--      tenant's own tag defaults FALSE, so nothing new starts provisioning by accident, and an
--      operator can flip either way in the admin System Tags editor (tag_handler accepts it).
--
--   2. `provisioning_log` — one row per ATTEMPT, written before the call and updated with the
--      outcome, so a refusal or a network failure can never be reported as a success. Same
--      "the row is the record" discipline as `webhook_delivery_log`. `status` is a small, fixed
--      vocabulary ('pending' | 'provisioned' | 'already_exists' | 'refused' | 'invalid' |
--      'failed' | 'unknown_app'), never the sibling's raw body.
--
-- IDEMPOTENT BY CONSTRUCTION. `ADD COLUMN IF NOT EXISTS` (the gate's rule 1: no unguarded ADD COLUMN)
-- and a `CREATE TABLE IF NOT EXISTS`. The seed UPDATE is data-driven, not a hardcoded id list: the
-- seven system tags are exactly the system tags that carry a mapping (every other system tag has
-- `source_app` NULL), and it is guarded by `provisions_account IS DISTINCT FROM true` so a re-run
-- moves zero rows and an operator's later choice of FALSE is never silently re-flipped by a boot.
--
-- FunnelSwift's OWN two tags (`FunnelSwift — Capture Free` / `… Kinetic Free`) are seeded TRUE like
-- the rest of the mapping, and the CALLER skips `source_app = 'funnelswift'` at run time: the lead's
-- owning workspace already IS the FunnelSwift account, so there is nothing to mint. Seeding them TRUE
-- keeps the admin panel honest — the switch reflects the mapping, and the skip is a rule of the
-- provisioner, stated in one place (src/app_provision.rs), not a hidden data difference.

ALTER TABLE tags
    ADD COLUMN IF NOT EXISTS provisions_account boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN tags.provisions_account IS
    'Applying this tag to a lead mints a FREE ACCOUNT in tags.source_app via the shared '
    'POST /api/v1/internal/provision-free-account contract (kanban t_847f9d63). Ignored when '
    'source_app is NULL or ''funnelswift''. Every attempt is recorded in provisioning_log.';

UPDATE tags
   SET provisions_account = true
 WHERE is_system = true
   AND source_app IS NOT NULL
   AND plan_slug IS NOT NULL
   AND provisions_account IS DISTINCT FROM true;

CREATE TABLE IF NOT EXISTS provisioning_log (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    lead_id         uuid REFERENCES leads(id) ON DELETE SET NULL,
    tenant_id       uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    tag_id          uuid REFERENCES tags(id) ON DELETE SET NULL,
    source_app      varchar(100) NOT NULL,
    plan_slug       varchar(255),
    idempotency_key varchar(255) NOT NULL,
    status          varchar(50) NOT NULL,
    http_status     integer,
    error_message   text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE provisioning_log IS
    'One row per free-account provisioning ATTEMPT driven by a lead tag (kanban t_847f9d63). '
    'Written as ''pending'' BEFORE the call and updated with the real outcome, so a refusal or an '
    'unreachable sibling is never stored (or shown) as a success. lead_id/tag_id are ON DELETE SET '
    'NULL: an account is never deleted because a lead or a tag was removed, so the record of minting '
    'it must survive them.';
COMMENT ON COLUMN provisioning_log.status IS
    'pending | provisioned (201) | already_exists (200) | refused (403) | invalid (422) | '
    'failed (network/5xx/anything else) | unknown_app (no base URL for source_app).';

CREATE INDEX IF NOT EXISTS idx_provisioning_log_lead
    ON provisioning_log (lead_id);
CREATE INDEX IF NOT EXISTS idx_provisioning_log_tenant
    ON provisioning_log (tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_provisioning_log_idem
    ON provisioning_log (idempotency_key);
