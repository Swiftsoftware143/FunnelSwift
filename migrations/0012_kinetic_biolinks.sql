-- Migration: Kinetic Bio-Links, Action Buttons, Source Tracking, and Event Analytics
-- Adds the kinetic cards system to FunnelSwift

-- 1. KINETIC CARD PROFILES (The Bio-Link Page)
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

CREATE INDEX IF NOT EXISTS idx_kinetic_cards_slug ON kinetic_cards(slug);
CREATE INDEX IF NOT EXISTS idx_kinetic_cards_user ON kinetic_cards(user_id);
CREATE INDEX IF NOT EXISTS idx_kinetic_cards_tenant ON kinetic_cards(tenant_id);

-- 2. DYNAMIC ACTION BUTTONS
CREATE TABLE IF NOT EXISTS kinetic_buttons (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
    label VARCHAR(100) NOT NULL,
    sort_order INT DEFAULT 0,
    action_type VARCHAR(20) NOT NULL, -- 'url', 'lead_form', 'sms'
    destination_url TEXT,
    target_tag_id UUID,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_kinetic_buttons_card ON kinetic_buttons(card_id);

-- 3. SOURCE PARAMETER MAP (src=ig → tag mapping)
CREATE TABLE IF NOT EXISTS kinetic_sources (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
    source_param VARCHAR(30) NOT NULL,
    target_tag_id UUID NOT NULL,
    UNIQUE(card_id, source_param)
);

CREATE INDEX IF NOT EXISTS idx_kinetic_sources_card ON kinetic_sources(card_id);

-- 4. EVENT ANALYTICS (Dashboard data)
CREATE TABLE IF NOT EXISTS lead_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    lead_id UUID,
    card_id UUID REFERENCES kinetic_cards(id) ON DELETE SET NULL,
    button_id UUID REFERENCES kinetic_buttons(id) ON DELETE SET NULL,
    event_type VARCHAR(20) NOT NULL, -- 'page_view', 'button_click', 'form_submit'
    source_param VARCHAR(30),
    ip_hash VARCHAR(64),
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lead_events_card ON lead_events(card_id);
CREATE INDEX IF NOT EXISTS idx_lead_events_tenant ON lead_events(tenant_id);
CREATE INDEX IF NOT EXISTS idx_lead_events_type ON lead_events(event_type);
CREATE INDEX IF NOT EXISTS idx_lead_events_source ON lead_events(source_param);
CREATE INDEX IF NOT EXISTS idx_lead_events_created ON lead_events(created_at DESC);
