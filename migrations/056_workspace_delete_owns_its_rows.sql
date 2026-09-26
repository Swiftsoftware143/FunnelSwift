-- Migration 056: a workspace DELETE must own every row that names it.
--
-- Card: kanban t_3bfda0b2 (found while proving t_8151c83f). Evidence:
-- /opt/swift/audits/fs-tenant-orphans-t_3bfda0b2/ (00..12 probes, each before/after pair).
--
-- MEASURED on the live `funnelswift` database, 2026-09-25 (never inferred from a row count):
--
--   * `kinetic_cards` had NO foreign key at all (pg_constraint, contype='f' -> 0 rows). The other
--     card children (kinetic_buttons, kinetic_sources, kinetic_qr_codes, kinetic_card_events,
--     kinetic_card_daily_stats, kinetic_card_locations) hang off the card with ON DELETE CASCADE,
--     so nothing cascaded from the workspace: the card survived its tenant.
--   * Census of columns named tenant_id/user_id: 20 carry no FK. 12 of them point at `tenants(id)`
--     and a workspace DELETE left the row behind: kinetic_cards, kinetic_qr_codes, funnels,
--     campaigns, checkout_sessions, ocr_scans, product_categories, tag_change_log, lead_events,
--     affiliate_selections, affiliate_users, webhook_delivery_log. Each was proven live with one
--     throwaway workspace + one row + `DELETE FROM tenants` (07-matrix.txt: SUCCEEDED, 1 row left).
--   * Two edges did not orphan — they BLOCKED the delete outright with 23503:
--     provider_keys.tenant_id and password_resets.user_id are both NO ACTION (confdeltype='a').
--     A workspace holding a provider key or a live reset token could not be deleted at all.
--
-- DECISION (recorded here because it is the load-bearing choice):
--   * The FK route, not an explicit purge in the handler. `tenant_id` IS the workspace ownership
--     pointer: every reader in this app scopes by it (list_cards WHERE tenant_id=$1 and friends),
--     so a row whose `tenant_id` names a workspace that no longer exists is unreachable content,
--     not preserved data. Declaring it in the schema keeps ONE source of truth and covers tables
--     the handler does not know about.
--   * ON DELETE CASCADE for every one of them. The SPA's own confirmation dialog already states
--     "This removes its users, cards and leads", and `users.tenant_id` /
--     `tenant_plan_subscriptions.tenant_id` / `leads.tenant_id` are already CASCADE. A tenant
--     delete is the destructive admin action; leaving a half-deleted workspace (cards gone, funnel
--     rows stranded) is the worse failure.
--   * The `user_id`-class columns are deliberately NOT given an FK, and that is a decision, not a
--     miss: they are historical "recorded owner" ids, the card/lead belongs to the WORKSPACE and
--     not to the member. CASCADE on `users` would let an admin removing one staff member destroy
--     the workspace's cards and leads; SET NULL is impossible (NOT NULL) without a schema change
--     that 25 of 27 live cards would still violate. The app already documents this at
--     src/handlers/kinetic_handler.rs:1339 and resolves a dangling owner to NULL by hand.
--   * The 3 300+ pre-existing orphan rows are LEFT ALONE (the card's own acceptance says existing
--     rows are not touched). Those tables get the constraint NOT VALID: Postgres skips the
--     one-time scan of existing rows but enforces the edge on every INSERT/UPDATE and fires the
--     cascade on every DELETE from now on. Counts, argued table by table in the audit report:
--     kinetic_cards.tenant_id 6, lead_events.tenant_id 3828, tag_change_log.tenant_id 4.
--
-- 16 edges, idempotent (each guarded by pg_constraint), no row is read, moved or deleted.

-- 1) The 12 missing `tenant_id -> tenants(id)` edges. `not_valid` marks the three whose table
--    already holds orphan rows (6 / 3828 / 4 measured) and therefore cannot be validated.
DO $mig$
DECLARE c record;
BEGIN
  FOR c IN
    SELECT * FROM (VALUES
      ('kinetic_cards',        true,  'no FK at all on the table; 6 live rows already orphaned'),
      ('lead_events',          true,  'no FK; 3828 live rows already orphaned'),
      ('tag_change_log',       true,  'no FK; 4 live rows already orphaned'),
      ('kinetic_qr_codes',     false, 'no FK at all on the table'),
      ('funnels',              false, 'no FK at all on the table'),
      ('campaigns',            false, 'no FK at all on the table'),
      ('checkout_sessions',    false, 'no FK at all on the table'),
      ('ocr_scans',            false, 'no FK at all on the table'),
      ('product_categories',   false, 'no FK at all on the table'),
      ('affiliate_selections', false, 'no FK at all on the table'),
      ('affiliate_users',      false, 'no FK at all on the table'),
      ('webhook_delivery_log', false, 'no direct FK; it only cascaded via webhooks')
    ) AS v(tbl, not_valid, why)
  LOOP
    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
       WHERE conname = c.tbl || '_tenant_id_fkey'
         AND conrelid = ('public.' || quote_ident(c.tbl))::regclass)
    THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE%s',
        c.tbl, c.tbl || '_tenant_id_fkey',
        CASE WHEN c.not_valid THEN ' NOT VALID' ELSE '' END);
    END IF;
  END LOOP;
END $mig$;

-- 2) The 2 NO ACTION edges that made the workspace delete 500 instead of cascading (measured
--    23503: provider_keys_tenant_id_fkey blocking `tenants`, password_resets_user_id_fkey blocking
--    the `users` cascade). Same class as CoreSwift-CRM's 12 (t_d8d1a175); the arm is the fix.
--    provider_keys holds the workspace's own BYOK credentials and password_resets a dead token —
--    neither is a reason to refuse retiring the workspace.
DO $mig$
DECLARE c record;
BEGIN
  FOR c IN
    SELECT * FROM (VALUES
      ('provider_keys',   'tenant_id'),
      ('password_resets', 'user_id')
    ) AS v(tbl, col)
  LOOP
    IF EXISTS (
      SELECT 1 FROM pg_constraint
       WHERE conname = c.tbl || '_' || c.col || '_fkey'
         AND conrelid = ('public.' || quote_ident(c.tbl))::regclass
         AND confdeltype <> 'c')
    THEN
      EXECUTE format('ALTER TABLE %I DROP CONSTRAINT %I',
                     c.tbl, c.tbl || '_' || c.col || '_fkey');
    END IF;
    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
       WHERE conname = c.tbl || '_' || c.col || '_fkey'
         AND conrelid = ('public.' || quote_ident(c.tbl))::regclass)
    THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (%I) REFERENCES %I(id) ON DELETE CASCADE',
        c.tbl, c.tbl || '_' || c.col || '_fkey', c.col,
        CASE c.col WHEN 'tenant_id' THEN 'tenants' ELSE 'users' END);
    END IF;
  END LOOP;
END $mig$;

-- 3) Two child pointers that hang off a workspace-owned parent and had no FK, so the parent's
--    cascade could not reach them (both measured as the only workspace-owned child of their parent
--    with a missing edge). Both columns are nullable and both fates are the same: the child is
--    unreadable without its parent (a tag assignment of a deleted tag, a click of a deleted link).
DO $mig$
DECLARE c record;
BEGIN
  FOR c IN
    SELECT * FROM (VALUES
      ('contact_tags',     'tag_id',  'tags'),
      ('affiliate_clicks', 'link_id', 'affiliate_links')
    ) AS v(tbl, col, parent)
  LOOP
    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
       WHERE conname = c.tbl || '_' || c.col || '_fkey'
         AND conrelid = ('public.' || quote_ident(c.tbl))::regclass)
    THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (%I) REFERENCES %I(id) ON DELETE CASCADE',
        c.tbl, c.tbl || '_' || c.col || '_fkey', c.col, c.parent);
    END IF;
  END LOOP;
END $mig$;
