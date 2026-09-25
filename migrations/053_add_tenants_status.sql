-- t_2a557c30 — the admin Tenants screen writes and renders a tenant `status`, but no statement,
-- column or reader had one: the screen (www/dashboard.js STN/RTN, www-app/index.html) posts
-- {name,email,slug,status} and prints `t.status`, while `list_tenants` reported the hard-coded
-- literal "active" and the DB had nowhere to keep the value — an admin could set "Inactive", get a
-- 200, and still see "active".
--
-- Verdict (one home): the screen is a TENANT list, not a users/account list, so `status` is an
-- additive nullable-free column on `tenants`:
--   * the row identity is `tenants.id` (Plan is `tenant_plan_subscriptions`, not a users column),
--   * the DELETE cascades to users/cards/leads, i.e. it retires a workspace,
--   * users are modelled separately (112 users over 126 tenants, 14 tenants with no user at all,
--     plus /api/v1/admin/tenants/:id/users for the real per-tenant user list),
--   * `tenants.is_visible` never existed on this table (it is `affiliates.is_visible`), so there is
--     no older flag this was supposed to be.
-- Default 'active' == the literal the list JSON hard-coded, so the 126 existing rows keep exactly
-- their current effective status. Additive: no statement that does not name `status` is affected.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';

-- Backstop for the two values the screen actually offers (Active/Inactive); the handler validates
-- first so a bad value is a 400, never a Postgres 23514.
ALTER TABLE tenants DROP CONSTRAINT IF EXISTS tenants_status_check;
ALTER TABLE tenants ADD CONSTRAINT tenants_status_check CHECK (status IN ('active', 'inactive'));
