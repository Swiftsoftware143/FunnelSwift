-- Migration: Add multi-tag support to web_to_lead_configs
ALTER TABLE web_to_lead_configs ADD COLUMN IF NOT EXISTS tag_ids UUID[] DEFAULT '{}';
ALTER TABLE web_to_lead_configs DROP COLUMN IF EXISTS tag_id;
ALTER TABLE web_to_lead_configs DROP COLUMN IF EXISTS tag_name;
CREATE INDEX IF NOT EXISTS idx_web_to_lead_tag_ids ON web_to_lead_configs USING GIN(tag_ids);
