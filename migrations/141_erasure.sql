-- 141_erasure.sql — a customer's OWN right to erasure (2026-11-24, fleet standard §3-§7).
--
-- WHY: `POST /api/v1/auth/close-account` ends access but keeps every row on purpose (a closure is a
-- STATE, migration 132). Nothing in this app could destroy a workspace's data on the OWNER's
-- request, so a GDPR/CCPA-style "delete my data" ask had to be done by hand in Postgres — the
-- failure mode the fleet standard forbids. This migration adds the two things erasure needs:
--
--   1. `account_erasures` — ONE audit row per erasure that SURVIVES the deletion (actor, when, how
--      many rows went, how many invoices were kept, and the workspace name the kept invoices belong
--      to). It carries no password, no token and no email address.
--   2. `funnelswift_erase_tenant(uuid, uuid)` — the purge itself, in SQL, so the Rust handler never
--      builds a statement at run time (gate rule 5d) and so the table list cannot drift from the
--      schema: it is read from the CATALOGUE at call time, which means a table added by a later
--      migration is purged by this same function without anybody remembering to add it.
--
-- WHAT IT KEEPS, and says so before it runs: billing documents required by tax law live in
-- `invoices`. Those rows are DETACHED from the workspace (tenant_id -> NULL) instead of deleted,
-- which is why `invoices.tenant_id` loses its NOT NULL here. Everything else in the workspace goes.
--
-- Child-first deletion without a hand-written table order: each pass tries every tenant table and
-- tolerates `foreign_key_violation` (a parent another row still points at); once a pass deletes
-- nothing, the remaining order problem cannot exist. Then the tenant row itself is deleted, which
-- CASCADEs the children this function could not reach, and a final census REFUSES to report success
-- if ANY row still carries the tenant id — so a future table that hangs off the tenant without a
-- `tenant_id` column cannot be silently left behind.

-- Billing docs outlive the workspace. The FK stays (NO ACTION): a NULL tenant_id is not a row the
-- constraint has to resolve, and the value is never re-pointed at a different workspace.
ALTER TABLE invoices ALTER COLUMN tenant_id DROP NOT NULL;

CREATE TABLE IF NOT EXISTS account_erasures (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The erased workspace, kept as an opaque reference only (no FK: the row it named is gone).
    tenant_ref        uuid NOT NULL,
    -- Who pressed the button. The user row is deleted with the workspace, so this is the id, never
    -- an email address or a name.
    actor_ref         uuid,
    -- The name the KEPT invoices are billed to; tax records have to identify the customer, and the
    -- invoices carry no name column of their own.
    tenant_name       text NOT NULL DEFAULT '',
    rows_deleted      bigint NOT NULL DEFAULT 0,
    tables_touched    integer NOT NULL DEFAULT 0,
    invoices_retained integer NOT NULL DEFAULT 0,
    created_at        timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_account_erasures_tenant_ref ON account_erasures (tenant_ref);

DROP FUNCTION IF EXISTS funnelswift_erase_tenant(uuid, uuid);
CREATE FUNCTION funnelswift_erase_tenant(p_tenant uuid, p_actor uuid DEFAULT NULL)
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
        (tenant_ref, actor_ref, tenant_name, rows_deleted, tables_touched, invoices_retained)
    VALUES
        (p_tenant, p_actor, coalesce(v_name, ''), v_total, v_touched, v_invoices);

    RETURN jsonb_build_object(
        'status', 'erased',
        'tenant_ref', p_tenant,
        'rows_deleted', v_total,
        'tables_touched', v_touched,
        'invoices_retained', v_invoices,
        'erased_at', now()
    );
END;
$$;
