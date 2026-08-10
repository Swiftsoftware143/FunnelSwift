-- FunnelSwift: Create additional missing tables referenced by handlers

CREATE TABLE IF NOT EXISTS product_categories (
    id UUID PRIMARY KEY,
    name VARCHAR(255),
    description TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS contact_tags (
    id UUID PRIMARY KEY,
    contact_id UUID,
    tag_id UUID,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS webhook_events (
    id UUID PRIMARY KEY,
    event_type VARCHAR(100),
    payload JSONB,
    status VARCHAR(50) DEFAULT 'pending',
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS feature_limits (
    id UUID PRIMARY KEY,
    plan_id UUID,
    feature_key VARCHAR(100),
    limit_value INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS site_settings (
    id UUID PRIMARY KEY,
    key VARCHAR(255) UNIQUE,
    value TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
