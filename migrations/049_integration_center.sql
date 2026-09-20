-- Integration Center catalogue + CoreSwift presets (fleet standard 2026-09-20)
--
-- R2: every app has a NATIVE CoreSwift integration and the data flows INBOUND (capture app to
-- CoreSwift hub). The canonical catalogue row is identical in every spoke; the preset is step 2
-- of the base-URL resolution order (provider_keys.base_url wins when set).

INSERT INTO available_providers (key, name, description, requires_base_url, requires_metadata, icon) VALUES
    ('coreswift',        'CoreSwift CRM',            'Push leads into CoreSwift CRM',              false, '[]'::jsonb, 'hub'),
    ('smtp',             'SMTP',                     'Send email through your own mail server',    false, '[]'::jsonb, 'mail'),
    ('sendiio',          'Sendiio',                  'Email + SMS delivery service',               false, '[]'::jsonb, 'mail'),
    ('mailgun',          'Mailgun',                  'Transactional email sending',                false, '[]'::jsonb, 'mail'),
    ('sendgrid',         'SendGrid',                 'Email delivery service',                     false, '[]'::jsonb, 'mail'),
    ('ocr',              'OCR service',              'Document text extraction for scans',         true,  '[]'::jsonb, 'scan'),
    ('kinetic',          'Kinetic Cards & Funnels',  'Funnel opt-ins, cards and mini funnels captured in FunnelSwift', false, '[]'::jsonb, 'card'),
    ('affiliate_payout', 'Affiliate Payouts',        'Affiliate commission payouts',               false, '[]'::jsonb, 'dollar')
ON CONFLICT (key) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    requires_base_url = EXCLUDED.requires_base_url,
    requires_metadata = EXCLUDED.requires_metadata,
    icon = EXCLUDED.icon;

-- Step 2 of the base-URL resolution order (standard, Hub contract).
-- Statement separators are kept out of the comments on purpose: some migration runners split
-- the file on the semicolon and would execute the next statement as its own chunk.
CREATE TABLE IF NOT EXISTS integration_provider_presets (
    key VARCHAR(64) PRIMARY KEY,
    base_url VARCHAR(512) NOT NULL DEFAULT '',
    is_active BOOLEAN NOT NULL DEFAULT true,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO integration_provider_presets (key, base_url, notes) VALUES
    ('coreswift', 'http://localhost:8084', 'CoreSwift hub on this box')
ON CONFLICT (key) DO UPDATE SET
    base_url = EXCLUDED.base_url,
    is_active = true,
    updated_at = NOW();
