-- 084_tenant_settings_config_secrets_sealed
--
-- WHY (kanban t_b040a78e, item 2 — the DB-level half of the t_a794cb09 / t_a65483ff / t_a8ee62bd
-- credential class in this app).
--
-- `tenant_settings` rows under `email_config` / `mailgun_config` / `smtp_config` carry a TENANT's
-- own mail credential, which `email_provider::resolve` replays against the provider (SMTP password,
-- Mailgun/SendGrid/Sendiio key). The tenant settings route (`PUT /api/v1/settings`) takes an
-- ARBITRARY `{key, value}` pair and used to bind the value straight into the row, so a tenant that
-- supplied `api_key` / `smtp_password` — or the legacy SMTP-password aliases `password` / `pass`
-- that `EmailConfig::from_json` also reads — had that credential stored PLAINTEXT at rest, and
-- `resolve` -> `send_via_smtp` / `send_mailgun` then USED it. Leaked AND live: measured on the
-- pre-fix deploy by /opt/swift/bin/email-config-enc-smoke.py funnelswift --phase before.
--
-- The write path now seals those fields with the same `enc:v1:` envelope the rest of the app uses,
-- the tenant's own GET opens the value BEFORE masking it, a masked round-trip restores the stored
-- credential, and `email_provider::seal_legacy_tenant_config_secrets` converges a row that arrives
-- plaintext from a restored dump. Until now nothing in the DATABASE refused a plaintext write.
--
-- This file is the store-level backstop for the writers nobody has written yet: it refuses a
-- plaintext credential in any of the four field names, keyed to the three keys `resolve` reads. It
-- is deliberately LOOSER than the Rust seal path — it says "empty or sealed", nothing about which
-- key sealed it — so the database can never refuse a write the application accepted.
--
-- SCOPE: the CHECK is key-scoped (`key NOT IN (…)`), so an unrelated `tenant_settings` row is
-- untouched, and `value->>'<field>'` on a JSONB scalar / missing field is NULL, which coalesce
-- turns into the empty string — i.e. "no credential here" — and passes.
--
-- NOT VALID on purpose: pre-existing rows are exempt (there are none today — measured 2026-10-02,
-- 0 rows carry any mail key, 9 carry `lead_stages`) so the self-heal block below validates
-- immediately, and every NEW write is checked from the first apply. A row that arrives plaintext
-- later (a restored pre-fix dump) is left insertable and is converged by
-- `email_provider::seal_legacy_tenant_config_secrets` at boot, then the boot half
-- `ensure_tenant_config_seal_guard` re-arms this constraint and VALIDATEs it once nothing unsealed
-- remains. That is what makes a hand-dropped guard self-healing instead of permanent.
--
-- Guard idiom: /opt/swift/fleet/templates/guard-constraint-not-valid.sql (fleet convention
-- t_c9b09cc1). Idempotent (DROP IF EXISTS + ADD … NOT VALID + guarded VALIDATE), so it is safe
-- under this app's ledger runner (`sqlx::migrate!`, src/db.rs) and re-runnable by hand.
--
-- Unlike ADASwift there is no guard-applier script to register the pair with — FunnelSwift has no
-- apply-funnelswift-guards.sh and is not in deploy-app.sh's matrix. The install path IS this file
-- (embedded in the binary at compile time) and `ensure_tenant_config_seal_guard` is the repair path
-- for a constraint that is dropped by hand or lost in a partial restore.

ALTER TABLE public.tenant_settings
    DROP CONSTRAINT IF EXISTS tenant_settings_config_secrets_sealed;

ALTER TABLE public.tenant_settings
    ADD CONSTRAINT tenant_settings_config_secrets_sealed
    CHECK (
        key NOT IN ('email_config','mailgun_config','smtp_config')
        OR (
               (coalesce(value->>'api_key','')       = '' OR value->>'api_key'       LIKE 'enc:v1:%')
           AND (coalesce(value->>'smtp_password','') = '' OR value->>'smtp_password' LIKE 'enc:v1:%')
           AND (coalesce(value->>'password','')      = '' OR value->>'password'      LIKE 'enc:v1:%')
           AND (coalesce(value->>'pass','')          = '' OR value->>'pass'          LIKE 'enc:v1:%')
        )
    ) NOT VALID;

DO $guard$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'tenant_settings_config_secrets_sealed'
          AND conrelid = 'public.tenant_settings'::regclass
          AND convalidated
    ) THEN
        BEGIN
            ALTER TABLE public.tenant_settings VALIDATE CONSTRAINT tenant_settings_config_secrets_sealed;
            RAISE NOTICE 'public.tenant_settings.tenant_settings_config_secrets_sealed validated: every existing row is compliant';
        EXCEPTION
            WHEN check_violation THEN
                RAISE WARNING 'public.tenant_settings.tenant_settings_config_secrets_sealed still NOT VALID: pre-existing rows hold an unsealed mail credential (the boot seal plus a re-run of this file validate it); new writes are still rejected';
        END;
    END IF;
END
$guard$;
