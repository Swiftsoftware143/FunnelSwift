-- 107: the agency row's two NOT NULL limit columns and their feature_limits overrides now state ONE number.
--
-- MEASURED 2026-10-17 against the live plan matrix. `plans` for `agency` reads
-- max_custom_domains=10 / max_team_members=1 — the two limits that are NOT NULL columns with their own
-- DEFAULT (plan_handler.rs says why the INSERT leaves them to the column defaults) — and every
-- customer-facing surface renders THOSE columns: the dashboard's "Your Plan" strip, the upgrade cards
-- and the admin plan editor. The plan's own `feature_limits` rows, which `features::resolved_limit`
-- reads FIRST, carried -1 (unlimited) for both. So a panel promising 10 domains / 1 seat sat on a gate
-- resolving "unlimited" for the same two keys.
--
-- WHY THIS DIRECTION (and not the other one). The two numbers are NOT equivalent for the operator, so
-- the alignment has to pick a side, and the side has to be the one that changes nothing a customer can
-- observe:
--
--   * `max_team_members` is INERT BY DECISION (kanban t_49e4cef9, 2026-10-02): no route raises a
--     workspace's user count, so nothing reads this limit on any plan. 1 and -1 behave identically;
--     writing the plan's own 1 keeps the sold promise and leaves the gate correct if an add-a-member
--     route is ever built.
--   * `max_custom_domains` cannot be exceeded on ANY plan, because the thing it meters is a SINGLETON:
--     `features::get_usage_count` counts `tenant_settings` rows with key='custom_domain' (UNIQUE
--     (tenant_id, key)) and `set_custom_domain` enforces the limit only on the FIRST claim
--     (`if !has_domain`). So the count is 0 or 1, and a second write REPLACES the one domain instead of
--     adding another. Any positive limit — 1, 10 or -1 — therefore allows exactly the same one domain.
--     Proved live on 2026-10-17: a probe workspace on this plan set a domain, set a second one, and the
--     workspace held exactly ONE `custom_domain` row containing the second value.
--
-- Aligning the panel UP to -1 would publish "unlimited custom domains" for a product that can hold one
-- domain per workspace — a bigger promise than the truth. Aligning the OVERRIDE to the plan's own number
-- changes no gate outcome on any plan (both values behave identically, per the two points above) and no
-- published number, so the panel keeps saying what David priced, and the disagreement the per-plan audit
-- reports is gone.
--
-- The deeper finding — that "10 custom domains" (and Suite's "3") is a collection the app cannot hold,
-- so those tiers really sell ONE domain — is a product decision (build multi-domain support, or restate
-- the plans as 1) and is NOT settled here; the numbers are left as priced.
--
-- Idempotent: once the two sides agree the WHERE matches no row, and the admin panel's per-plan override
-- lever still wins the moment it writes a value that differs.
UPDATE feature_limits f
   SET limit_value = p.max_custom_domains
  FROM plans p
 WHERE f.plan_id = p.id
   AND p.slug = 'agency'
   AND f.feature_key = 'max_custom_domains'
   AND f.limit_value IS DISTINCT FROM p.max_custom_domains;

UPDATE feature_limits f
   SET limit_value = p.max_team_members
  FROM plans p
 WHERE f.plan_id = p.id
   AND p.slug = 'agency'
   AND f.feature_key = 'max_team_members'
   AND f.limit_value IS DISTINCT FROM p.max_team_members;
