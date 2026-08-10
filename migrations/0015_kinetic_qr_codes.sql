-- Migration: Kinetic QR Codes — per-card QR generation with plan-based limits
-- Free: 1 QR code, Upgraded: unlimited

CREATE TABLE IF NOT EXISTS kinetic_qr_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    card_id UUID NOT NULL REFERENCES kinetic_cards(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    is_active BOOLEAN DEFAULT TRUE NOT NULL,
    download_count INT DEFAULT 0 NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_kinetic_qr_tenant ON kinetic_qr_codes(tenant_id);
CREATE INDEX IF NOT EXISTS idx_kinetic_qr_card ON kinetic_qr_codes(card_id);
CREATE INDEX IF NOT EXISTS idx_kinetic_qr_user ON kinetic_qr_codes(user_id);
