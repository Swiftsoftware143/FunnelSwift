-- 125 — the TERM a workspace is actually billed on (2026-11-10).
--
-- THE GAP THIS CLOSES. A customer picks a plan AND a term on the Upgrade screen. The ask records
-- both (`plan_requests.billing_cycle`, migration 124 — 'monthly' | 'yearly'), the operator's queue
-- reads both, and the customer's own screen prints the term they chose. But `tenant_plan_subscriptions`
-- carried NO term at all: `set_active_plan` wrote (id, tenant_id, plan_id, status, start_date) and
-- nothing else, so the moment the operator marked the ask `done`, the term the customer agreed to was
-- dropped on the floor and every reader fell back to `plans.billing_cycle`, which is a CATALOGUE
-- default ('month' on every row of every install) rather than this customer's choice. A customer who
-- chose "Yearly — 2 months free" was therefore recorded, forever, as monthly — and on an install with
-- no payment provider, where the operator invoices from these rows, that is the wrong invoice.
--
-- WHY 'month'/'year' AND NOT 'monthly'/'yearly'. The column is read beside `plans.billing_cycle`
-- (the console prints "price / <term>", and `me()` serves the plans row), whose vocabulary is
-- 'month' | 'year'. The plan_requests table keeps its own human-facing 'monthly' | 'yearly' wording;
-- the plan-request fulfilment path maps between the two so the two screens never disagree.
--
-- SAFE TO RE-RUN: this fleet's runner applies unrecorded files at every boot, so every statement is
-- idempotent (ADD COLUMN IF NOT EXISTS, a pg_constraint probe around the CHECK).
ALTER TABLE tenant_plan_subscriptions
    ADD COLUMN IF NOT EXISTS billing_cycle varchar(10) NOT NULL DEFAULT 'month';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'tenant_plan_subscriptions_billing_cycle_check'
    ) THEN
        ALTER TABLE tenant_plan_subscriptions
            ADD CONSTRAINT tenant_plan_subscriptions_billing_cycle_check
            CHECK (billing_cycle IN ('month', 'year'));
    END IF;
END
$$;

COMMENT ON COLUMN tenant_plan_subscriptions.billing_cycle IS
    'The term THIS subscription is charged on (month|year), from the plan request the customer filed. Falls back to plans.billing_cycle only for rows written before migration 125.';
