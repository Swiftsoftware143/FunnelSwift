-- Per-plan affiliate payout rate (admin-adjustable).
-- Affiliates are regular users; their effective payout % is the rate
-- configured on the plan their account is subscribed to.
ALTER TABLE plans ADD COLUMN IF NOT EXISTS commission_rate NUMERIC(5,2) NOT NULL DEFAULT 20.0;
