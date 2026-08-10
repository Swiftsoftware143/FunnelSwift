-- Migration 0012b: Add layout_blocks JSONB to kinetic_cards for dynamic mini-pages
-- This replaces the static design columns with a dynamic LayoutBlock system

-- Add layout_blocks column for dynamic section-based page rendering
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS layout_blocks JSONB DEFAULT '[]'::jsonb;

-- Add meta_description for SEO
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS meta_description TEXT;

-- Add logo_url for tenant branding
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS logo_url TEXT;

-- Rename bio to tagline for clarity (keep bio as alias)
ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS tagline TEXT;

-- Create function to build default layout blocks from legacy fields
-- This will be used by the data migration script
