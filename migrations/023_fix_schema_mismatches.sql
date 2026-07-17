-- FunnelSwift: Fix schema column mismatches between code and migrations

-- Users table: add columns queried by handlers
ALTER TABLE users ADD COLUMN IF NOT EXISTS first_name VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_name VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS username VARCHAR(255);
ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_url TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified BOOLEAN DEFAULT false;
ALTER TABLE users ADD COLUMN IF NOT EXISTS source_of_entry VARCHAR(100);

-- Leads table: add columns queried by handlers
ALTER TABLE leads ADD COLUMN IF NOT EXISTS first_name VARCHAR(255);
ALTER TABLE leads ADD COLUMN IF NOT EXISTS last_name VARCHAR(255);

-- API Keys table: add full_key column (migration has key_hash, prefix but no full_key)
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS full_key TEXT;

-- Plans table: add purchase_url column
ALTER TABLE plans ADD COLUMN IF NOT EXISTS purchase_url TEXT;

-- Tenants table: add columns queried by handlers
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS affiliate_code VARCHAR(100);
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS referred_by UUID;

-- Affiliate Links table: add columns queried by handlers
ALTER TABLE affiliate_links ADD COLUMN IF NOT EXISTS product_id UUID;
ALTER TABLE affiliate_links ADD COLUMN IF NOT EXISTS target_app VARCHAR(100);
ALTER TABLE affiliate_links ADD COLUMN IF NOT EXISTS target_url TEXT;
ALTER TABLE affiliate_links ADD COLUMN IF NOT EXISTS code VARCHAR(100);
ALTER TABLE affiliate_links ADD COLUMN IF NOT EXISTS is_active BOOLEAN DEFAULT true;
