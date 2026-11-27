-- A plan request that has been ACKNOWLEDGED to the customer can never be deleted.
--
-- WHY (measured 2026-11-26 23:19-23:20Z): a live probe minted the real EMI Mold Removal
-- customer's session, pressed Upgrade for Agency / Scale on the production database, and the
-- refusal arm filed `plan_requests` row 6c51df70-4135-4018-a42a-02dcb3920727 and mailed BOTH the
-- customer ("We have your FunnelSwift plan request - Agency / Scale") and the operator. One minute
-- later the same probe deleted that row as "residue". The customer held a written acknowledgement
-- of a request that no longer existed anywhere and the operator's queue showed nothing.
--
-- TWO DURABLE GUARANTEES, both in the database rather than in a script:
--
--   1. `acknowledged_at` is stamped BEFORE the receipt leaves the mail layer, so an acknowledgement
--      can only exist for a row that is already committed. The send funnel refuses to mail when the
--      stamp cannot be written (see `email::send_plan_request_received_email`).
--   2. A BEFORE DELETE trigger refuses to remove a stamped row. Probe clean-up of an unacknowledged
--      (harness/withheld) request still works; a request the customer was told about cannot vanish.
--
-- `plan_request_events` is the append-only trace: created / acknowledged / deleted, each with the
-- snapshot the mail was built from. A deletion that somehow bypasses the guard still leaves a row
-- here, so "the request disappeared" is always answerable from this database.

ALTER TABLE plan_requests
    ADD COLUMN IF NOT EXISTS acknowledged_at TIMESTAMPTZ;

COMMENT ON COLUMN plan_requests.acknowledged_at IS
    'When the customer was mailed the receipt. Stamped before the send; a stamped row cannot be deleted.';

CREATE TABLE IF NOT EXISTS plan_request_events (
    id          BIGSERIAL PRIMARY KEY,
    request_id  UUID NOT NULL,
    -- created | acknowledged | deleted
    event       TEXT NOT NULL CHECK (event IN ('created', 'acknowledged', 'deleted')),
    tenant_id   UUID,
    plan_slug   TEXT,
    plan_name   TEXT,
    requested_by TEXT,
    status      TEXT,
    created_at  TIMESTAMPTZ,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE plan_request_events IS
    'Append-only history of every plan request: created, acknowledged and deleted. Survives the row.';

CREATE INDEX IF NOT EXISTS plan_request_events_request
    ON plan_request_events (request_id, recorded_at);

CREATE OR REPLACE FUNCTION plan_requests_record_event() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        INSERT INTO plan_request_events
            (request_id, event, tenant_id, plan_slug, plan_name, requested_by, status, created_at)
        VALUES
            (NEW.id, 'created', NEW.tenant_id, NEW.plan_slug, NEW.plan_name, NEW.requested_by,
             NEW.status, NEW.created_at);
        RETURN NEW;
    ELSIF TG_OP = 'UPDATE' THEN
        -- Only the first acknowledgement is an event; a repeated receipt is not a new promise.
        IF NEW.acknowledged_at IS NOT NULL AND OLD.acknowledged_at IS NULL THEN
            INSERT INTO plan_request_events
                (request_id, event, tenant_id, plan_slug, plan_name, requested_by, status, created_at)
            VALUES
                (NEW.id, 'acknowledged', NEW.tenant_id, NEW.plan_slug, NEW.plan_name,
                 NEW.requested_by, NEW.status, NEW.created_at);
        END IF;
        RETURN NEW;
    ELSE
        INSERT INTO plan_request_events
            (request_id, event, tenant_id, plan_slug, plan_name, requested_by, status, created_at)
        VALUES
            (OLD.id, 'deleted', OLD.tenant_id, OLD.plan_slug, OLD.plan_name, OLD.requested_by,
             OLD.status, OLD.created_at);
        RETURN OLD;
    END IF;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS plan_requests_record_event ON plan_requests;
CREATE TRIGGER plan_requests_record_event
    AFTER INSERT OR UPDATE OR DELETE ON plan_requests
    FOR EACH ROW EXECUTE FUNCTION plan_requests_record_event();

CREATE OR REPLACE FUNCTION plan_requests_refuse_acknowledged_delete() RETURNS trigger AS $$
BEGIN
    IF OLD.acknowledged_at IS NOT NULL THEN
        RAISE EXCEPTION
            'plan request % was acknowledged to % on % and cannot be deleted',
            OLD.id, OLD.requested_by, OLD.acknowledged_at
            USING ERRCODE = 'restrict_violation',
                  HINT = 'Close it instead of deleting it: the customer holds a written acknowledgement.';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS plan_requests_refuse_acknowledged_delete ON plan_requests;
CREATE TRIGGER plan_requests_refuse_acknowledged_delete
    BEFORE DELETE ON plan_requests
    FOR EACH ROW EXECUTE FUNCTION plan_requests_refuse_acknowledged_delete();
