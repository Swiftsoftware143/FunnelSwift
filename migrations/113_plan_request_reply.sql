-- The operator can answer a plan request, not only move its state (2026-10-19, cron `default`).
--
-- WHY: `PUT /api/v1/admin/plan-requests/:id` could set `status` and nothing else, so a customer
-- whose ask was `declined` or `contacted` read a bare chip on their Upgrade screen with no sentence
-- explaining it — and the operator had no way to say "we will call you Tuesday" or "this plan needs a
-- custom domain first". The customer's screen already promises "You will see the state change here";
-- this column is what makes that promise useful.
--
-- Additive and idempotent: a replayed boot is a no-op and nothing existing is touched.
ALTER TABLE plan_requests ADD COLUMN IF NOT EXISTS reply TEXT NOT NULL DEFAULT '';
ALTER TABLE plan_requests ADD COLUMN IF NOT EXISTS replied_at TIMESTAMPTZ;
