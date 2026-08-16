-- Migration 031: affiliate commission metadata (upgrade-event traceability + idempotency)
-- Upgrade events from the other Swift apps carry an event_id; store it plus the plan
-- context on the commission so repeated deliveries never double-credit the affiliate.
ALTER TABLE affiliate_commissions ADD COLUMN IF NOT EXISTS metadata JSONB;
