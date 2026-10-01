-- 071: a referred customer's plan movements, DATED — upgrade, downgrade, and upgrade again.
--
-- David, 2026-10-01: *"have we verified that a free user who upgrades in app gets shown as upgraded in
-- the affiliates dashboard? As well as the admins dashboard? So that way the affiliate gets credit?
-- Can we also make sure it's dated when a person upgrades or downgrades."*
--
-- Measured before this migration, and every part of that was missing:
--   * `set_active_plan` — the ONLY writer of a tenant's plan in this app — touched nothing in the
--     affiliate system. A FunnelSwift customer upgrading in app therefore credited nobody: no
--     commission moved, no dashboard changed, nothing was recorded.
--   * A commission was inserted as `pending` and NOTHING in the codebase ever set it to anything
--     else, so even a correctly detected upgrade by a sibling app left the affiliate's money
--     permanently "pending".
--   * Downgrades were not detected at all — while the affiliate guide served from this same module
--     already told affiliates: *"a lead can upgrade, downgrade, and upgrade again months later, and
--     you're credited each time."* The download promised a flow the backend did not have.
--   * No date was recorded anywhere for either event, so even the state that did exist could not be
--     explained or audited.
--
-- This table is the DATED timeline: one row per plan movement, for every referred tenant, whether the
-- movement came from this app or was reported by a sibling through the internal webhook. It is
-- append-only on purpose — the history is the record, and the current state is derived from it, so a
-- later re-upgrade can never erase the earlier downgrade.

CREATE TABLE IF NOT EXISTS affiliate_plan_movements (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The referred lead this movement belongs to. NULL when the tenant cannot be traced back to a
    -- lead (an account that arrived on its own): the movement is still recorded, credited to nobody,
    -- because "system" is a real answer and hiding it would make the programme's own revenue
    -- unaccountable.
    lead_id      uuid,
    -- The tenant whose plan moved. NULL when the movement was reported by a SIBLING app: that
    -- customer's workspace lives in the sibling's own database, so this app never learns its id.
    -- The timeline is driven by lead_id + affiliate_id, which is what both dashboards read.
    tenant_id    uuid,
    -- Denormalised on purpose: the affiliate who earns from this tenant, resolved AT THE TIME of the
    -- movement, so a later rename or deactivation cannot rewrite history.
    affiliate_id varchar(64),
    movement     text NOT NULL,          -- 'upgrade' | 'downgrade' | 'start' | 'plan_change'
    from_plan    text,
    to_plan      text NOT NULL,
    from_price   numeric,
    to_price     numeric NOT NULL DEFAULT 0,
    -- Set by whoever reported the movement (this app, or the sibling that sent it), so a retry of the
    -- same event cannot produce a second timeline entry.
    event_key    text,
    occurred_at  timestamptz NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_apm_tenant    ON affiliate_plan_movements (tenant_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_apm_affiliate ON affiliate_plan_movements (affiliate_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_apm_lead      ON affiliate_plan_movements (lead_id, occurred_at DESC);
-- one row per reported event; a NULL event_key (a locally observed movement) is always allowed
CREATE UNIQUE INDEX IF NOT EXISTS idx_apm_event_key ON affiliate_plan_movements (event_key)
    WHERE event_key IS NOT NULL;

-- The commission's current state, dated. `status` was already there but had exactly one value in
-- practice; these two columns are what make "when did this become real, and when did it stop" an
-- answerable question.
ALTER TABLE affiliate_commissions ADD COLUMN IF NOT EXISTS earned_at   timestamptz;
ALTER TABLE affiliate_commissions ADD COLUMN IF NOT EXISTS reversed_at timestamptz;

-- Existing rows: a commission that has been paid was certainly earned, and the row's own timestamp is
-- the only honest date available for it.
UPDATE affiliate_commissions SET earned_at = COALESCE(paid_at, created_at)
 WHERE earned_at IS NULL AND status IN ('earned', 'paid');

COMMENT ON TABLE affiliate_plan_movements IS
'Dated timeline of every plan movement for a referred tenant (upgrade/downgrade/re-upgrade), with the affiliate credited at the time. Written by set_active_plan (in-app) and the internal upgrade webhook (siblings).';
