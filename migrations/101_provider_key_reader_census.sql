-- 101_provider_key_reader_census.sql
-- FunnelSwift: the OTHER seven Integration Center rows that accept and store a provider credential no
-- code path in this app can spend. Migration 100 closed six (openai, deepseek, nexweave, sam_gov,
-- usaspending, telnyx) with available_providers.coming_soon. This file decides the rest, PER ROW,
-- from a fresh census of every provider_keys reader in src/. Kanban t_2a019b99.
--
-- THE CENSUS (2026-10-11, off-peak, this tree -- `grep -rn provider_keys src/ --include=*.rs`):
--   a query on provider_keys is run in exactly TWO places, and both pin the provider:
--     src/coreswift.rs:90 + src/handlers/coreswift_integration_handler.rs:200   provider 'coreswift'
--     src/handlers/incentiveswift_handler.rs:41                                 provider 'incentiveswift'
--   plus src/handlers/provider_keys_handler.rs (its own CRUD and read-back) and src/features.rs (a
--   COUNT for the max_integrations limit). Nothing else in the tree reads the table, so of the
--   fifteen catalogue rows exactly TWO have a reader. Every row below is one of the OTHER seven.
--
-- (a) mailgun / sendgrid / sendiio / smtp -- THE CREDENTIAL IS READ, FROM A DIFFERENT DOOR.
--   src/email_provider.rs::resolve reads tenant_settings keys email_config / mailgun_config /
--   smtp_config and then the global admin_settings.email row. It never touches provider_keys. The one
--   operator surface that writes the global row is the admin console's "Email Templates -> Email
--   Provider (System Email)" card (POST /api/v1/admin/email-config). A tenant's own email_config has
--   an API (PUT /api/v1/settings, sealed by migration 084) and no screen in either console.
--   So a customer pasting a Mailgun key into the Integration Center stored a credential nothing ever
--   reads, while email kept sending through the Email Provider setting. These four are NOT
--   coming_soon - email works today - so they get credential_note instead: the card names where the
--   credential actually lives and stops asking for a provider key.
--
-- (b) affiliate_payout / kinetic / ocr -- NO READER ANYWHERE, and none coming.
--   affiliate_payout: this app RECORDS payouts in its own table (handlers/affiliate_payout_handler.rs,
--     migration 021) and calls no payout provider - no key, no token, no outbound request.
--   kinetic: Kinetic Cards are this app's OWN feature (kinetic_cards / kinetic_buttons / kinetic_qr_codes).
--     There is no third-party Kinetic API, so there is no key to hold.
--   ocr: the OCR feature (src/handlers/ocr.rs) calls the host Tesseract service with a token read from
--     the CONTAINER ENV (OCR_SERVICE_TOKEN / OCR_SERVICE_URL), never from a tenant credential, so a
--     customer key for it is unreadable by construction.
--   All three are flagged coming_soon, the same shape migration 100 used for the first six.
--
-- LABEL, DON'T HIDE (David's rule): every row stays visible in both consoles. coming_soon rows render
-- as informational Coming Soon cards, credential_note rows as a card that names the real door, and
-- BOTH are refused by upsert_provider_key with a 400 that names the reason - so the free-text provider
-- form can no longer store a credential for any row this app cannot spend.
--
-- REVERSAL (one line each): a coming_soon row returns to connectable with
--   UPDATE available_providers SET coming_soon = false WHERE key = 'ocr'
-- and a credential_note row with
--   UPDATE available_providers SET credential_note = NULL WHERE key = 'smtp'
-- Each then needs the missing half named - a real dispatch arm that reads that provider's key.
--
-- ADDITIVE + IDEMPOTENT: ADD COLUMN IF NOT EXISTS, and both UPDATEs are no-ops once applied.
-- provider_keys holds ZERO rows on production, so no stored customer credential is affected.
-- No BEGIN/COMMIT and no semicolons inside comments - the runner wraps the file.

ALTER TABLE available_providers
    ADD COLUMN IF NOT EXISTS credential_note text;

UPDATE available_providers
   SET credential_note = 'the Email Provider setting'
 WHERE key IN ('mailgun', 'sendgrid', 'sendiio', 'smtp');

UPDATE available_providers
   SET coming_soon = true
 WHERE key IN ('affiliate_payout', 'kinetic', 'ocr');
