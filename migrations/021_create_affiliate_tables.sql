-- FunnelSwift: Create missing affiliate tracking tables
-- These tables are referenced by handlers but were created out-of-band.

CREATE TABLE IF NOT EXISTS affiliate_clicks (
    id UUID PRIMARY KEY,
    link_id UUID,
    visitor_id VARCHAR(255),
    ip_address INET,
    user_agent TEXT,
    clicked_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS affiliate_users (
    id UUID PRIMARY KEY,
    tenant_id UUID,
    user_id UUID,
    code VARCHAR(100) UNIQUE,
    commission_rate DECIMAL(5,2) DEFAULT 0,
    total_earned DECIMAL(12,2) DEFAULT 0,
    total_paid DECIMAL(12,2) DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS affiliate_conversions (
    id UUID PRIMARY KEY,
    click_id UUID,
    affiliate_user_id UUID,
    customer_id UUID,
    amount DECIMAL(12,2),
    commission DECIMAL(12,2),
    status VARCHAR(50) DEFAULT 'pending',
    converted_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS affiliate_payouts (
    id UUID PRIMARY KEY,
    affiliate_user_id UUID,
    amount DECIMAL(12,2),
    method VARCHAR(100),
    status VARCHAR(50) DEFAULT 'pending',
    paid_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS affiliate_selections (
    id UUID PRIMARY KEY,
    tenant_id UUID,
    product_id UUID,
    code VARCHAR(100),
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS affiliate_products (
    id UUID PRIMARY KEY,
    name VARCHAR(255),
    description TEXT,
    price DECIMAL(12,2),
    commission_rate DECIMAL(5,2),
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
