-- Migration 124: a plan request names the BILLING TERM the customer chose (2026-11-10).
--
-- WHY THIS EXISTS (measured live 2026-11-10): the Upgrade screen has offered a "Yearly — 2 months
-- free" switch since 2026-10-24 and prices every paid tier from `plans.annual_price` (Capture
-- Starter 89, Kinetic Pro 89, Suite 290, Agency 790). But the request path that IS this
-- installation's only buying path (`POST /api/v1/plan-requests` — no payment provider is
-- configured, so `checkout/create` refuses every purchase) dropped the term on the floor:
-- `requestPlan()` posted `{plan_slug}` alone and this table had nowhere to put a term. A customer
-- who pressed "Yearly — 2 months free" filed a request the operator could only read as monthly,
-- and would be invoiced for a term they had not chosen.
--
-- `billing_cycle` carries the term the customer picked: 'monthly' or 'yearly'. The DEFAULT is what
-- every existing row and every caller that names no term already means, so those are unchanged.

ALTER TABLE plan_requests ADD COLUMN IF NOT EXISTS billing_cycle text NOT NULL DEFAULT 'monthly';

-- The customer's own list, the operator's queue and the duplicate check all read
-- (tenant, plan, term) while an ask is still open, so index exactly that.
CREATE INDEX IF NOT EXISTS idx_plan_requests_open_term
    ON plan_requests (tenant_id, plan_slug, billing_cycle)
    WHERE status IN ('open', 'contacted');
