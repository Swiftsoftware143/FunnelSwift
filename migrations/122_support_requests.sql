-- Migration 122: support requests — a signed-in workspace can ASK FOR HELP and read the answer.
--
-- WHY THIS EXISTS (measured 2026-11-06, cron `default`): every other app in the fleet gives a
-- signed-in customer an in-product way to reach the team (support tickets on ADASwift /
-- IncentiveSwift / MissedCall, tickets on WorkflowSwift and CoreSwift). FunnelSwift has NONE:
-- there is no support surface in the tenant console and no address printed anywhere a signed-in
-- customer can see, while this app's own refusal text tells them to "contact support" — and
-- `plan_request_handler`'s module docs already record the same hole ("the screen's own failure
-- text pointed at a 'support ticket', which this app does not have").
--
-- `plan_requests` cannot carry this: its columns are a PLAN ask and the operator's queue resolves
-- it by moving the workspace onto a plan. A general help request has a subject, a message, and a
-- written answer, so it gets its own table with the same tenant scoping and the same
-- operator-queue shape (one row per ask, status + reply are what the customer reads back).
--
-- Statuses are deliberately three:
--   open      the customer is waiting on us
--   answered  the operator replied; `reply` carries the sentence the customer reads
--   closed    handled, nothing further needed (the operator can close without a reply)
-- A row is never deleted by the operator: the history is the record, and the customer's own list
-- shows the state. Deleting a workspace cascades its requests away with it.

CREATE TABLE IF NOT EXISTS support_requests (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id      uuid,
    requested_by text NOT NULL DEFAULT '',
    subject      text NOT NULL,
    body         text NOT NULL DEFAULT '',
    status       text NOT NULL DEFAULT 'open',
    reply        text NOT NULL DEFAULT '',
    replied_at   timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT support_requests_status_check CHECK (status IN ('open', 'answered', 'closed')),
    CONSTRAINT support_requests_subject_len CHECK (char_length(subject) BETWEEN 1 AND 120),
    CONSTRAINT support_requests_body_len CHECK (char_length(body) <= 4000),
    CONSTRAINT support_requests_reply_len CHECK (char_length(reply) <= 4000)
);

-- The customer's own read is always scoped by tenant and ordered newest-first.
CREATE INDEX IF NOT EXISTS support_requests_tenant_idx
    ON support_requests (tenant_id, created_at DESC);

-- The operator's queue is "everything still open, newest first".
CREATE INDEX IF NOT EXISTS support_requests_open_idx
    ON support_requests (status, created_at DESC);
