-- 151: a funnel's public address belongs to the WORKSPACE that owns the funnel, not the whole install.
--
-- Measured 2026-11-28 (kanban t_c50db970, found while proving the kinetic-card fix t_89f6de9c):
-- `GET /funnel/zz-probe-funnel-capture` with `Host: c.zaarhub.com` answered 200 with that page's
-- title — a funnel owned by the Swift Admin's Workspace, served on ZaarHub's branded domain —
-- because `funnels.slug` carried a UNIQUE index on the slug ALONE (`uq_funnels_slug`) while a
-- funnel belongs to one workspace (`funnels.tenant_id`). Two consequences, both live-class:
--   * the public resolver looked a funnel up by slug only, so ANY workspace's page could answer on
--     ANY other workspace's branded domain;
--   * two workspaces could never both own `/spring-promo` — the second create was silently given
--     `spring-promo-2`, i.e. a link nobody chose, because the install-wide uniqueness forced it.
--
-- This is the exact rule migration 150 applied to kinetic cards, applied to the sibling surface:
-- uniqueness becomes (tenant_id, slug), so the address is scoped like the row it names.
--
-- The code side (`handlers/funnel_handler.rs`): the public page, its view counter and the public
-- submit arm resolve the workspace that CLAIMED the request host first (through the kinetic
-- handler's `request_scope`, so the rule has one owner) and look the slug up inside it. Only when
-- that workspace holds no such slug does the account-wide row answer — every link printed before
-- this change keeps working — and that fallback is logged at WARN, never silent.
--
-- The slug-alone uniqueness is a bare UNIQUE INDEX on the live install (pg_indexes: CREATE UNIQUE
-- INDEX uq_funnels_slug). The constraint arm is dropped too so an install that created it as a
-- table constraint converges to the same shape.

ALTER TABLE funnels DROP CONSTRAINT IF EXISTS uq_funnels_slug;
DROP INDEX IF EXISTS uq_funnels_slug;

-- A duplicate (tenant_id, slug) pair would make this fail, and it SHOULD fail loudly: the migration
-- then aborts, the boot records the error and /api/health answers 503, which the fleet watchdog
-- alerts on. Silently picking a winner would rename a customer's live link.
CREATE UNIQUE INDEX IF NOT EXISTS funnels_tenant_slug_key
    ON funnels (tenant_id, slug);

-- The public resolver still filters by slug alone (`WHERE slug = $1` + a scope preference), so the
-- plain slug index stays exactly as it was: `idx_funnels_slug`.
