-- Migration 061: the rows left behind by already-deleted workspaces are REMOVED, and the three
-- workspace edges that migration 056 had to add NOT VALID are VALIDATEd.
--
-- Card: kanban t_4dd9e87b (found while closing t_3bfda0b2). Evidence:
-- /opt/swift/audits/fs-orphan-purge-t_4dd9e87b/ (01-pre.txt before, 0N-post.txt after) and the
-- archive of every removed row in .../archive/ (sha256 in .../archive/SHA256SUMS).
--
-- MEASURED on the live `funnelswift` database, 2026-09-26, before this file ran (01-pre.txt):
--
--   kinetic_cards       6 orphans of 27   lead_events 3828 of 4151   tag_change_log 4 of 6
--   leads 0 orphans of 56                 users 0 orphans of 112     contact_tags 0 of 0
--   the 6 cards and their dead workspaces:
--     fcf1cf95… david-giraudy-gc   tenant 3e3a9325… (Giraudy Capital)  3617 lead_events
--     4c53211c… giraudy-capital    tenant 3e3a9325… (Giraudy Capital)    94 lead_events
--     306d72a7… david              tenant a6bc421f… (SwiftSoftware dup)  14 lead_events
--     9928c3bb… test               tenant a6bc421f… (SwiftSoftware dup)   0
--     9e5b19f4… gate-test-1789919428  tenant 3b6e6c12… (fixture)           0
--     ac7d46a2… gf-1789919428         tenant 8fdb0bdf… (fixture)           0
--   and three more dead workspaces named only by lead_events / tag_change_log:
--     fcaad346… 131 lead_events, 2e04695b… 1 tag_change_log row, acba0545… 1 tag_change_log row.
--   All 6 cards were still PUBLIC: GET /c/<slug> -> 200 for all six (01-pre-http.txt, local and
--   public edge), because render_card looks the card up by slug with a LEFT JOIN on tenants and
--   never asks whether the workspace still exists. Not one of them could be listed, edited,
--   unpublished or deleted in the product -- every reader scopes by tenant_id (list_cards
--   WHERE tenant_id = $1) and the workspace they scope by is gone.
--
-- DECISION (the load-bearing choice, recorded here): PURGE the 3838 rows. Not adopt.
--   * `tenant_id` IS the workspace ownership pointer -- migration 056's own recorded decision, and
--     this file is the same decision applied to the rows 056 deliberately left alone. A row whose
--     tenant_id names a workspace that does not exist is unreachable content, not preserved data.
--   * A workspace DELETE is the product's destructive admin action; its own SPA dialog states
--     "This removes its users, cards and leads". These rows are that delete's unfinished work, so
--     removing them makes the product's stated behaviour true for the rows it already happened to.
--   * ADOPT would NOT fix the defect this card reports. A holding tenant has no users, so nobody
--     could log into it: the card would still be public content nobody can turn off -- the same
--     hole, cosmetically closed -- and it would re-attribute 3838 rows to a workspace that never
--     existed. A purge is a state change; an adoption is a fabrication.
--   * What is actually destroyed, measured (01-pre.txt): 3823 page_view + 4 form_submit +
--     1 button_click events, and 4 tag_change_log rows of two dead workspaces. ZERO `leads` rows
--     are orphaned in any direction (0 of 56) -- the CRM records, which belong to live workspaces,
--     are untouched by this file.
--   * Reversible BY HAND, not by the app: every removed row is archived under
--     /opt/swift/audits/fs-orphan-purge-t_4dd9e87b/archive/ as CSV plus a pg_dump of the three
--     tables (fs_orphan_tables_pre.dump), and the two named dead workspaces (Giraudy Capital,
--     the SwiftSoftware duplicate) exist verbatim in the nightly dump
--     /opt/swift/backups/pg-nightly/20260910-023001/funnelswift.sql.gz (extracted to
--     archive/dead_tenants_rows.txt). Restoring means re-creating those tenant rows first, then
--     replaying the archived rows -- i.e. the "adopt" option is still available later, on demand.
--   * Because the rows go, the three workspace edges 056 added NOT VALID over them can be
--     VALIDATEd here: after this file no kinetic_cards/lead_events/tag_change_log row points at a
--     missing workspace, so `ALTER TABLE … VALIDATE CONSTRAINT` succeeds and the schema is fully
--     declared (056's own report recorded NOT VALID as "deliberate until those rows are dealt
--     with"; this is that).
--
-- Idempotent: the DELETEs are keyed on a workspace that is missing, so a second run removes 0 rows,
-- and each VALIDATE is guarded by pg_constraint.convalidated. No BEGIN/COMMIT (sqlx wraps each
-- migration in its own transaction).

DO $mig$
DECLARE
  pre_cards  int; pre_le int; pre_tcl int;
  del_cards  int; del_le int; del_tcl int;
BEGIN
  SELECT count(*) INTO pre_cards FROM kinetic_cards WHERE tenant_id NOT IN (SELECT id FROM tenants);
  SELECT count(*) INTO pre_le    FROM lead_events   WHERE tenant_id NOT IN (SELECT id FROM tenants);
  SELECT count(*) INTO pre_tcl   FROM tag_change_log WHERE tenant_id NOT IN (SELECT id FROM tenants);
  RAISE NOTICE '061: orphans before purge: kinetic_cards %, lead_events %, tag_change_log %',
               pre_cards, pre_le, pre_tcl;

  -- Order: the events first, then the audit log, then the cards. `lead_events.card_id` is
  -- ON DELETE SET NULL, so deleting the cards first would null 3725 pointers on rows that are
  -- about to go anyway -- the same end state, one pointless write more.
  DELETE FROM lead_events WHERE tenant_id NOT IN (SELECT id FROM tenants);
  GET DIAGNOSTICS del_le = ROW_COUNT;

  DELETE FROM tag_change_log WHERE tenant_id NOT IN (SELECT id FROM tenants);
  GET DIAGNOSTICS del_tcl = ROW_COUNT;

  -- The card children (kinetic_buttons, kinetic_sources, kinetic_qr_codes, kinetic_card_events,
  -- kinetic_card_daily_stats, kinetic_card_locations) cascade off this delete. Measured before the
  -- purge: all six cards had 0 children of every kind (01-pre.txt §B).
  DELETE FROM kinetic_cards WHERE tenant_id NOT IN (SELECT id FROM tenants);
  GET DIAGNOSTICS del_cards = ROW_COUNT;

  RAISE NOTICE '061: purged kinetic_cards % / lead_events % / tag_change_log % row(s) belonging to workspaces that no longer exist',
               del_cards, del_le, del_tcl;

  SELECT count(*) INTO pre_cards FROM kinetic_cards WHERE tenant_id NOT IN (SELECT id FROM tenants);
  SELECT count(*) INTO pre_le    FROM lead_events   WHERE tenant_id NOT IN (SELECT id FROM tenants);
  SELECT count(*) INTO pre_tcl   FROM tag_change_log WHERE tenant_id NOT IN (SELECT id FROM tenants);
  RAISE NOTICE '061: orphans after purge: kinetic_cards %, lead_events %, tag_change_log %',
               pre_cards, pre_le, pre_tcl;
END $mig$;

-- 056 left these three NOT VALID because the rows above existed (Postgres skips the one-time scan
-- of existing rows but still enforces the edge on every INSERT/UPDATE). With the rows gone the scan
-- succeeds, so the edges become fully valid constraints rather than permanently-dodged ones.
-- Guarded twice: only when the constraint exists, and only when it is not already validated.
DO $mig$
DECLARE c record; left_cards int; left_le int; left_tcl int;
BEGIN
  SELECT count(*) INTO left_cards FROM kinetic_cards  WHERE tenant_id NOT IN (SELECT id FROM tenants);
  SELECT count(*) INTO left_le    FROM lead_events    WHERE tenant_id NOT IN (SELECT id FROM tenants);
  SELECT count(*) INTO left_tcl   FROM tag_change_log WHERE tenant_id NOT IN (SELECT id FROM tenants);
  IF left_cards <> 0 OR left_le <> 0 OR left_tcl <> 0 THEN
    RAISE NOTICE '061: NOT validating: % / % / % orphan row(s) still present, VALIDATE would fail',
                 left_cards, left_le, left_tcl;
    RETURN;
  END IF;

  FOR c IN
    SELECT * FROM (VALUES
      ('kinetic_cards',  'kinetic_cards_tenant_id_fkey'),
      ('lead_events',    'lead_events_tenant_id_fkey'),
      ('tag_change_log', 'tag_change_log_tenant_id_fkey')
    ) AS v(tbl, con)
  LOOP
    IF EXISTS (
      SELECT 1 FROM pg_constraint
       WHERE conname = c.con
         AND conrelid = ('public.' || quote_ident(c.tbl))::regclass
         AND contype = 'f'
         AND NOT convalidated)
    THEN
      EXECUTE format('ALTER TABLE %I VALIDATE CONSTRAINT %I', c.tbl, c.con);
      RAISE NOTICE '061: validated %', c.con;
    END IF;
  END LOOP;
END $mig$;
