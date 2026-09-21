-- 051_payment_providers_target_software_api_key_encrypted_at_rest.sql
--
-- Second and third credential columns in FunnelSwift (same defect class as 050, which fixed
-- provider_keys for kanban t_81cd0b4d). Before this migration both write paths bound the raw
-- request value straight into the column, so a customer-supplied secret sat in the clear at rest
-- and would fall out of any database dump or leaked backup:
--
--   payment_providers.api_key  POST /api/v1/payment-providers  (Stripe / payment secret keys)
--                              and the list endpoint echoed the raw value back to the client
--   target_software.api_key    POST /api/v1/target-software + POST/PUT /api/v1/integration-targets
--                              (outbound webhook signing keys, IncentiveSwift bearer)
--
-- The app now encrypts both before it writes, through the SAME choke point as provider_keys
-- (src/security/provider_key_crypto.rs): AES-256 via pgcrypto, master key held ONLY in the process
-- environment (PROVIDER_KEY_ENC_SECRET), stored as 'enc:v1:' + base64 ciphertext. Read paths
-- decrypt and publish `api_key_masked` only.
--
-- These constraints are the regression guard: a future writer that forgets to encrypt FAILS CLOSED
-- at the database instead of silently storing a plaintext credential. NULL and the empty string
-- stay allowed so "no credential stored" keeps the representation the tables already used.
--
-- NOT VALID by design: a row written before today stays exempt so the app keeps reading it, while
-- every NEW insert/update is checked. At the time of writing BOTH tables hold zero credentials
-- (payment_providers 0 rows, target_software 1 row with an empty key), so there is no legacy
-- plaintext to backfill and the constraint is validated immediately below. If a plaintext row is
-- ever found before this file runs, encrypt it first with
--     /opt/swift/bin/backfill_provkeys.sh-style UPDATE ... pgp_sym_encrypt(...)
-- otherwise the VALIDATE statement aborts the migration (and therefore the boot).
--
-- The no-semicolon rule applies to every comment above: the deploy staging path and the migration
-- runner treat the statement separator specially, so a comment must never contain one.

ALTER TABLE payment_providers DROP CONSTRAINT IF EXISTS payment_providers_api_key_encrypted;

ALTER TABLE payment_providers ADD CONSTRAINT payment_providers_api_key_encrypted CHECK (api_key IS NULL OR api_key = '' OR api_key LIKE 'enc:v1:%') NOT VALID;

ALTER TABLE target_software DROP CONSTRAINT IF EXISTS target_software_api_key_encrypted;

ALTER TABLE target_software ADD CONSTRAINT target_software_api_key_encrypted CHECK (api_key IS NULL OR api_key = '' OR api_key LIKE 'enc:v1:%') NOT VALID;

ALTER TABLE payment_providers VALIDATE CONSTRAINT payment_providers_api_key_encrypted;

ALTER TABLE target_software VALIDATE CONSTRAINT target_software_api_key_encrypted;
