-- Migration: create kinetic_cards BEFORE 0015 needs it (ordering repair, no schema change).
--
-- WHY THIS FILE EXISTS (kanban t_f7341aa9, measured 2026-09-26)
--   0015_kinetic_qr_codes.sql declares
--       card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE
--   but that table was only created by 0017_kinetic_biolinks.sql, two files LATER. sqlx applies
--   migrations in numeric version order, so a from-zero database died at 0015 with
--       error returned from database: relation "kinetic_cards" does not exist
--   and, because src/db.rs deliberately treats a migration error as non-fatal, the process still
--   booted and answered readiness 200 on a partial schema: 12 of 44 migrations applied,
--   29 of 58 tables, nothing from 0016 onward.
--
--   The CREATE below is the kinetic_cards statement of 0017, verbatim (same columns, same order,
--   same defaults); 0018 / 0024 / 0028 / 0042 / 0057 keep adding their columns on top exactly as
--   they already do. 0017 is NOT edited: its version is recorded in _sqlx_migrations on every
--   live database and sqlx aborts the entire run with VersionMismatch when an APPLIED file's
--   checksum changes, so editing 0015/0017 would break production instead of fixing a fresh
--   install. This file instead takes the VACANT version 4 (4 and 8 are unused; a gap is legal —
--   see /opt/swift/fleet/migration-version-check.sh) so it sorts before 0015.
--
--   On a live database this file is a no-op (the table already exists, IF NOT EXISTS skips), and
--   its only effect is one new _sqlx_migrations row.
--
--   The three indexes on kinetic_cards stay where they are declared (0017) — this file owns the
--   referent of 0015's foreign key and nothing else.

CREATE TABLE IF NOT EXISTS kinetic_cards (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    slug VARCHAR(50) UNIQUE NOT NULL,
    title VARCHAR(100) NOT NULL,
    bio TEXT,
    avatar_url TEXT,
    template_type VARCHAR(30) DEFAULT 'default',
    -- Video Configuration (Premium)
    video_provider VARCHAR(20),
    video_id VARCHAR(50),
    -- Design / Colors
    bg_color VARCHAR(7) DEFAULT '#121212',
    text_color VARCHAR(7) DEFAULT '#FFFFFF',
    accent_color VARCHAR(7) DEFAULT '#3B82F6',
    button_bg_color VARCHAR(7) DEFAULT '#1F2937',
    button_text_color VARCHAR(7) DEFAULT '#FFFFFF',
    -- Social Links
    instagram_url TEXT,
    facebook_url TEXT,
    twitter_url TEXT,
    youtube_url TEXT,
    linkedin_url TEXT,
    tiktok_url TEXT,
    -- Active / Deactivated
    is_active BOOLEAN DEFAULT TRUE NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL
);
