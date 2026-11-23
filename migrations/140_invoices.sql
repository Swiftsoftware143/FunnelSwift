-- 140_invoices.sql — the operator's own invoice ledger for FunnelSwift (2026-11-23).
--
-- WHY: card payments are OFF on this installation (`payment_providers` holds zero rows), so a plan
-- REQUEST is the buying path and the operator's `billing_instructions` words tell a customer how to
-- send money. What was missing is the RECORD: nothing in this app stored what a customer owed or
-- what they had paid, so the operator's only book was his own memory and an inbox — and a customer
-- had nothing to point at. A sibling app (ADASwift, migration 036) already carries this table; this
-- is the FunnelSwift side of the same rail, so a workspace that asks to buy can be BILLED today
-- with no payment provider, no key and no card.
--
-- One row per invoice, raised by the operator from the admin console. `status` is
-- pending | paid | void. `paid_at` is stamped when the operator marks it paid and cleared if the
-- invoice is reopened or voided, so "paid" and "has a paid date" can never disagree.
--
-- `plan_id` is nullable: an invoice can cover something that is not a catalogue plan (a setup fee,
-- a one-off), and refusing to record it would push that money back into the inbox.
--
-- Idempotent by construction: `sqlx::migrate!` applies each file once and records its checksum, and
-- every statement is IF NOT EXISTS, so a re-run on a database that already carries the table is a
-- no-op.

CREATE TABLE IF NOT EXISTS invoices (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   uuid NOT NULL REFERENCES tenants(id),
    plan_id     uuid REFERENCES plans(id),
    amount      numeric(10, 2) NOT NULL,
    status      character varying(20) NOT NULL DEFAULT 'pending',
    note        text,
    due_date    timestamp with time zone,
    paid_at     timestamp with time zone,
    created_at  timestamp with time zone NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS invoices_tenant_created_idx
    ON invoices (tenant_id, created_at DESC);

COMMENT ON TABLE invoices IS
    'Invoices the operator raises for a customer while card payments are off. pending | paid | void.';
