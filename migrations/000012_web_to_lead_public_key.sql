-- Migration: Add public_key for embed script (no manual API key needed)
ALTER TABLE web_to_lead_configs ADD COLUMN IF NOT EXISTS public_key UUID DEFAULT gen_random_uuid();
CREATE UNIQUE INDEX IF NOT EXISTS idx_web_to_lead_public_key ON web_to_lead_configs(public_key);
