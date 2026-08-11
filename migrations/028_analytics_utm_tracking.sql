-- Migration 028: Kinetic Card Analytics + UTM + Location Tracking
-- Covers all 5 card types universally

-- A) UTM columns on kinetic_cards (for URL generation)
ALTER TABLE kinetic_cards 
  ADD COLUMN IF NOT EXISTS utm_source TEXT,
  ADD COLUMN IF NOT EXISTS utm_medium TEXT,
  ADD COLUMN IF NOT EXISTS utm_campaign TEXT,
  ADD COLUMN IF NOT EXISTS utm_content TEXT,
  ADD COLUMN IF NOT EXISTS utm_term TEXT;

-- B) Card events table (one row per view/click/share/tap - any card type)
CREATE TABLE IF NOT EXISTS kinetic_card_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  event_type TEXT NOT NULL CHECK (event_type IN ('view', 'click', 'share', 'qr_scan', 'nfc_tap', 'button_click', 'link_click', 'form_submit')),
  event_label TEXT,  -- which element: "cta_call", "cta_book", "link_instagram", "button_download", etc.
  
  -- UTM tracking (captured from URL params on load)
  utm_source TEXT,
  utm_medium TEXT,
  utm_campaign TEXT,
  utm_content TEXT,
  utm_term TEXT,
  
  -- Referrer & device
  referrer_url TEXT,
  user_agent TEXT,
  device_type TEXT,      -- 'mobile', 'tablet', 'desktop'
  browser_family TEXT,   -- 'chrome', 'safari', 'firefox', etc.
  os_family TEXT,        -- 'ios', 'android', 'windows', 'macos', 'linux'
  
  -- Location (from IP — anonymous)
  country TEXT,
  region TEXT,
  city TEXT,
  timezone TEXT,
  ip_hash TEXT,          -- SHA-256 of IP (anonymous, can't reverse)
  
  -- Session
  session_id TEXT,
  
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_card_events_card ON kinetic_card_events(card_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_card_events_tenant ON kinetic_card_events(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_card_events_type ON kinetic_card_events(card_id, event_type, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_card_events_utm ON kinetic_card_events(utm_source, utm_medium, utm_campaign, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_card_events_country ON kinetic_card_events(country, created_at DESC);

-- C) Daily card rollup (aggregated for fast dashboard queries)
CREATE TABLE IF NOT EXISTS kinetic_card_daily_stats (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  stat_date DATE NOT NULL DEFAULT CURRENT_DATE,
  
  views INT NOT NULL DEFAULT 0,
  clicks INT NOT NULL DEFAULT 0,
  shares INT NOT NULL DEFAULT 0,
  qr_scans INT NOT NULL DEFAULT 0,
  unique_visitors INT NOT NULL DEFAULT 0,
  
  -- UTM breakdown
  utm_source TEXT,
  utm_medium TEXT,
  utm_campaign TEXT,
  
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  
  UNIQUE(card_id, stat_date, utm_source, utm_medium, utm_campaign)
);

CREATE INDEX IF NOT EXISTS idx_card_daily_card ON kinetic_card_daily_stats(card_id, stat_date);
CREATE INDEX IF NOT EXISTS idx_card_daily_tenant ON kinetic_card_daily_stats(tenant_id, stat_date);

-- D) Location aggregation
CREATE TABLE IF NOT EXISTS kinetic_card_locations (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  
  country TEXT NOT NULL,
  region TEXT,
  city TEXT,
  view_count INT NOT NULL DEFAULT 1,
  last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
  
  UNIQUE(card_id, country, COALESCE(region,''), COALESCE(city,''))
);
