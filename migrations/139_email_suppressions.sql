-- 139_email_suppressions.sql — a do-not-mail list this app can actually honour (kanban B214, 2026-11-21).
--
-- WHY. FunnelSwift had no place to record "this person asked us to stop", and no unsubscribe link
-- that landed anywhere. Every other app in the fleet (IncentiveSwift, WorkflowSwift, CoreSwift) has
-- carried this table and a gate in its send path since the compliance work landed there; this file
-- brings the same shape here so a recipient's opt-out, or an operator's decision to stop mailing a
-- hard-bounced address, is obeyed by the sender instead of being advice.
--
-- SHAPE. Keyed by (tenant_id, address, mail_class) so suppression is per ACCOUNT and per CLASS:
-- one workspace's opt-out never silences another workspace's mail to the same person, and an
-- opt-out from notices never blocks a password reset. `mail_class` is 'notification' for the
-- stoppable class and 'all' for a blanket stop (used when an address hard-bounces).
--
-- Idempotent: guarded so a re-run (the boot runner applies unrecorded files every start) cannot
-- fail on an existing table, and the unique key is declared inside CREATE TABLE.
CREATE TABLE IF NOT EXISTS email_suppressions (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  uuid NOT NULL,
    address    text NOT NULL,
    mail_class text NOT NULL DEFAULT 'notification',
    reason     text NOT NULL DEFAULT '',
    source     text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT email_suppressions_unique UNIQUE (tenant_id, address, mail_class)
);

CREATE INDEX IF NOT EXISTS email_suppressions_lookup
    ON email_suppressions (tenant_id, address);
