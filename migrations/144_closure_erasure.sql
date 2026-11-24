-- 144_closure_erasure.sql — the CLOSURE-to-ERASURE TIMER (fleet standard §2, kanban t_d01b2a79).
--
-- WHY: the standard promises "A closure auto-erases after 30 days unless support reverses it." The
-- close door (migration 132) records `users.closed_at` and moves the session line, but NOTHING read
-- that column to destroy anything: the only readers were the login/forgot-password refusals and
-- read-only console filters. So a cancelled workspace's leads, funnels, cards, inbox and media stayed
-- forever while the customer guide and the standard promised deletion at day 30. That is the exact
-- failure mode the standard exists to stop — a promise the code does not keep.
--
-- This migration puts the POLICY in ONE SQL definition so the window, the skip list and the purge
-- cannot drift apart:
--
--   1. `account_erasures.reason` — an erasure row now says WHY it happened: 'owner-request' (the
--      Danger-zone button) or 'closure-Nd' (this timer). Existing rows are owner requests, which is
--      what the column default records.
--   2. `funnelswift_erase_tenant(uuid, uuid, text)` — the SAME purge as migration 141, with the reason
--      argument added. Nothing else about what it deletes, keeps or proves changes.
--   3. `funnelswift_closures_due(days)` — the CENSUS: the workspaces whose closure window has run out.
--      This is where every rule lives: the 30-day floor, "every person in the workspace is closed",
--      and the four workspaces that are never touched (operator, system/fixture, portfolio).
--
-- The RUNNER is `ops/closure_erasure.py`, installed in root's crontab daily off-peak. It reads the
-- census ONCE and then calls the purge once per workspace, each in its own statement, so one stubborn
-- workspace cannot roll back the others and a second run resumes where the first stopped (an erased
-- workspace is gone from the census, so re-running is a no-op by construction).
--
-- REVERSAL (the standard's "unless support reverses it"): support clears the closure —
-- `UPDATE users SET closed_at = NULL, updated_at = now() WHERE tenant_id = <id>;` — and the next census
-- no longer sees the workspace, because the window is measured from `users.closed_at` at run time.
-- `docs/closure-erasure.md` carries the operator recipe and what must be re-issued afterwards.

-- 1. The audit row learns WHY. The default keeps every pre-existing row honest: all of them were
--    owner requests, because the timer did not exist before this migration.
ALTER TABLE account_erasures ADD COLUMN IF NOT EXISTS reason text NOT NULL DEFAULT 'owner-request';

COMMENT ON COLUMN account_erasures.reason IS
  'owner-request = the customer pressed Delete account and data; closure-Nd = the automatic closure timer (funnelswift_closures_due).';

-- The timer scans this column every run; make it an index rather than a heap walk.
CREATE INDEX IF NOT EXISTS idx_users_closed_at ON users (closed_at) WHERE closed_at IS NOT NULL;

-- 2. ONE purge, two doors. DROP-then-CREATE rather than CREATE OR REPLACE: a third parameter is a
--    NEW signature, so the two-argument function would survive beside it and make every two-bind call
--    (`SELECT funnelswift_erase_tenant($1, $2)` in auth::handlers::erase_data) AMBIGUOUS. Dropping the
--    old one inside this migration's transaction leaves exactly one definition with the defaults.
DROP FUNCTION IF EXISTS funnelswift_erase_tenant(uuid, uuid);
CREATE FUNCTION funnelswift_erase_tenant(
    p_tenant uuid,
    p_actor uuid DEFAULT NULL,
    p_reason text DEFAULT 'owner-request'
)
RETURNS jsonb
LANGUAGE plpgsql
AS $$
DECLARE
    v_name       text := '';
    v_invoices   integer := 0;
    v_deleted    bigint := 0;
    v_total      bigint := 0;
    v_pass       integer := 0;
    v_progress   bigint := 0;
    v_n          bigint := 0;
    v_raw        bigint := 0;
    v_left       bigint := 0;
    v_touched    integer := 0;
    v_tenant_row integer := 0;
    r            record;
BEGIN
    SELECT name INTO v_name FROM tenants WHERE id = p_tenant;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'erasure: workspace % does not exist', p_tenant
            USING ERRCODE = 'no_data_found';
    END IF;

    -- KEEP the billing documents, detached from the workspace that is about to disappear.
    UPDATE invoices SET tenant_id = NULL WHERE tenant_id = p_tenant;
    GET DIAGNOSTICS v_invoices = ROW_COUNT;

    -- Purge every table the CATALOGUE says carries this tenant, child-first by retry.
    LOOP
        v_pass := v_pass + 1;
        v_progress := 0;
        FOR r IN
            SELECT c.relname AS tbl
              FROM pg_class c
              JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public'
               AND c.relkind = 'r'
               AND c.relname NOT IN ('invoices', 'account_erasures')
               AND EXISTS (
                     SELECT 1 FROM information_schema.columns ic
                      WHERE ic.table_schema = 'public'
                        AND ic.table_name = c.relname
                        AND ic.column_name = 'tenant_id'
                   )
        LOOP
            BEGIN
                EXECUTE format('DELETE FROM %I WHERE tenant_id = $1', r.tbl) USING p_tenant;
                GET DIAGNOSTICS v_n = ROW_COUNT;
                IF v_n > 0 THEN
                    v_progress := v_progress + v_n;
                    v_touched := v_touched + 1;
                END IF;
            EXCEPTION WHEN foreign_key_violation THEN
                -- A child this pass could not delete first; the next pass takes it.
                NULL;
            END;
        END LOOP;
        EXIT WHEN v_progress = 0;
        v_total := v_total + v_progress;
        IF v_pass >= 12 THEN
            RAISE EXCEPTION 'erasure: workspace % still has linked rows after % passes',
                p_tenant, v_pass;
        END IF;
    END LOOP;

    -- The workspace row itself: CASCADEs whatever hung off it without a tenant_id column.
    DELETE FROM tenants WHERE id = p_tenant;
    GET DIAGNOSTICS v_tenant_row = ROW_COUNT;
    IF v_tenant_row = 0 THEN
        RAISE EXCEPTION 'erasure: workspace % vanished mid-purge', p_tenant;
    END IF;
    v_total := v_total + v_tenant_row;

    -- PROVE the purge, do not assume it: no row anywhere may still name the workspace.
    FOR r IN
        SELECT c.relname AS tbl
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind = 'r'
           AND c.relname NOT IN ('invoices', 'account_erasures')
           AND EXISTS (
                 SELECT 1 FROM information_schema.columns ic
                  WHERE ic.table_schema = 'public'
                    AND ic.table_name = c.relname
                    AND ic.column_name = 'tenant_id'
               )
    LOOP
        EXECUTE format('SELECT count(*) FROM %I WHERE tenant_id = $1', r.tbl)
            INTO v_raw USING p_tenant;
        v_left := v_left + v_raw;
    END LOOP;
    IF v_left > 0 THEN
        RAISE EXCEPTION 'erasure: % row(s) still name workspace % after the purge', v_left, p_tenant;
    END IF;

    INSERT INTO account_erasures
        (tenant_ref, actor_ref, tenant_name, rows_deleted, tables_touched, invoices_retained, reason)
    VALUES
        (p_tenant, p_actor, coalesce(v_name, ''), v_total, v_touched, v_invoices,
         coalesce(NULLIF(btrim(p_reason), ''), 'owner-request'));

    RETURN jsonb_build_object(
        'status', 'erased',
        'tenant_ref', p_tenant,
        'rows_deleted', v_total,
        'tables_touched', v_touched,
        'invoices_retained', v_invoices,
        'reason', coalesce(NULLIF(btrim(p_reason), ''), 'owner-request'),
        'erased_at', now()
    );
END;
$$;

COMMENT ON FUNCTION funnelswift_erase_tenant(uuid, uuid, text) IS
  'The one workspace purge (fleet standard §4). Invoices are DETACHED and kept; every other tenant row is deleted; refuses to report success while a row still names the workspace. p_actor is NULL for an automatic closure erasure.';

-- 3. THE CENSUS. A workspace is due when:
--      * it has at least one person (a workspace with no users records no closure — nothing to honour),
--      * EVERY person in it is closed (a teammate still signed in is never erased under them),
--      * no person in it is a platform operator (an install's own console is not a customer),
--      * the LAST closure is at least `days` old — measured from the most recent `closed_at`, so the
--        clock starts when the last person leaves, never when the first one did,
--      * it is not the system seed, a fleet fixture the tenancy canary looks up by name, a
--        probe-harness workspace, or anything the portfolio registry / name list owns.
--
-- The window is FLOORED at 30 days, whatever a caller asks for: the standard's promise is 30 days, so
-- no script argument, cron typo or casual `SELECT` can shorten the customer's restore window. (That
-- floor is why the runner's `--days` exists only for proving the 29/31-day boundary on a scratch copy.)
CREATE OR REPLACE FUNCTION funnelswift_closures_due(p_days integer DEFAULT 30)
RETURNS TABLE (
    tenant_id      uuid,
    workspace      text,
    closed_users   integer,
    last_closed_at timestamptz
)
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    v_days integer := GREATEST(coalesce(p_days, 30), 30);
BEGIN
    RETURN QUERY
    SELECT t.id,
           COALESCE(NULLIF(btrim(t.name), ''), t.slug),
           uu.closed_users,
           uu.last_closed_at
      FROM tenants t
      JOIN LATERAL (
             SELECT count(*)::integer AS users_total,
                    count(*) FILTER (WHERE u.closed_at IS NOT NULL)::integer AS closed_users,
                    max(u.closed_at) AS last_closed_at,
                    bool_or(u.role IN ('admin', 'super_admin')) AS has_operator
               FROM users u
              WHERE u.tenant_id = t.id
           ) uu ON TRUE
     WHERE uu.users_total > 0
       AND uu.closed_users = uu.users_total
       AND NOT uu.has_operator
       AND uu.last_closed_at <= now() - make_interval(days => v_days)
       AND lower(t.slug) NOT LIKE 'probe-canary-%'
       AND lower(t.slug) NOT LIKE 'system%'
       AND (t.probe_harness IS NULL OR btrim(t.probe_harness) = '')
       AND NOT EXISTS (
             SELECT 1
               FROM portfolio_companies p
              WHERE p.tenant_id = t.id
                 OR p.account_tenant_id = t.id
                 OR lower(p.name) = lower(t.name)
                 OR lower(p.slug) = lower(t.slug)
           )
       AND NOT (
             lower(t.name || ' ' || coalesce(t.email, '')) LIKE '%swiftimpact%'
          OR lower(t.name || ' ' || coalesce(t.email, '')) LIKE '%zaarhub%'
          OR lower(t.name || ' ' || coalesce(t.email, '')) LIKE '%giraudy%'
           )
     ORDER BY uu.last_closed_at;
END;
$$;

COMMENT ON FUNCTION funnelswift_closures_due(integer) IS
  'Workspaces whose closure window (at least 30 days, from the last person''s closed_at) has run out and that the fleet keeps by rule: operator, system/fixture/probe-harness and portfolio workspaces are never returned. The runner ops/closure_erasure.py erases what this returns.';
