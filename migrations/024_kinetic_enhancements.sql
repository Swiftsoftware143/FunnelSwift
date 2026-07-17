-- Migration 024: Add kinetic cards enhancements — password gates, consent, GA tracking, etc.

ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS password_hash TEXT;
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS consent_required BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS ga_tracking_id TEXT;
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS cookie_policy_url TEXT;
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS age_gate_type TEXT; -- 'none', 'age_18', 'age_21', 'custom'
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS age_gate_message TEXT;
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS consent_decline_redirect TEXT;
