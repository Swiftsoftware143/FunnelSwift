CREATE TABLE IF NOT EXISTS linkedin_auths (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    encrypted_cookies TEXT NOT NULL,
    cookie_expires_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'expired', 'disconnected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id)
);

-- Index for fast lookups by user
CREATE INDEX IF NOT EXISTS idx_linkedin_auths_user_id ON linkedin_auths(user_id);
CREATE INDEX IF NOT EXISTS idx_linkedin_auths_status ON linkedin_auths(status);

-- Auto-update updated_at
CREATE OR REPLACE FUNCTION update_linkedin_auths_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_linkedin_auths_updated_at ON linkedin_auths;
CREATE TRIGGER trg_linkedin_auths_updated_at
    BEFORE UPDATE ON linkedin_auths
    FOR EACH ROW
    EXECUTE FUNCTION update_linkedin_auths_updated_at();
