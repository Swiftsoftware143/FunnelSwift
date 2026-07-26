-- Migration 042: Add theme support to kinetic cards

ALTER TABLE kinetic_cards ADD COLUMN IF NOT EXISTS theme VARCHAR(50);
CREATE INDEX IF NOT EXISTS idx_kinetic_cards_theme ON kinetic_cards(theme);
