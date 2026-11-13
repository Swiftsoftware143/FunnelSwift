-- 130 — API keys: minted with the account, RETRIEVABLE, auditable (kanban t_7863696b).
--
-- David, 2026-11-11: the "shown once, never again" model is withdrawn. A key is stored twice:
--   * key_hash   — hex SHA-256, the ONLY thing the bearer resolver reads;
--   * key_sealed — the same key sealed with this app's provider-key envelope (`enc:v1:…`,
--                  pgp_sym_encrypt under PROVIDER_KEY_ENC_SECRET), used ONLY to display it back
--                  to its owner. Authentication never decrypts; reveal never authenticates.
--
-- Replaces the `api_keys` table dropped by migration 090 (kanban t_5a3c2d9c), whose hash was never
-- compared against a presented bearer and whose plaintext was never retrievable — the credential
-- surface this card restores with both halves.
CREATE TABLE IF NOT EXISTS api_keys (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name         text NOT NULL,
    -- First characters of the plaintext key, kept for a human "which key is this?" label.
    key_prefix   text NOT NULL,
    -- hex SHA-256 of the full plaintext token (64 chars). Never the token itself.
    key_hash     text NOT NULL UNIQUE,
    -- The same key sealed at rest with this app's provider-key envelope, ONLY for display.
    key_sealed   text NOT NULL,
    created_by   uuid,
    created_at   timestamp without time zone NOT NULL DEFAULT now(),
    last_used_at timestamp without time zone,
    -- Soft revocation: resolving a key also requires `revoked_at IS NULL`, so a revoked key
    -- stops authenticating immediately while its row (and its audit trail) stays.
    revoked_at   timestamp without time zone
);

CREATE INDEX IF NOT EXISTS idx_api_keys_tenant ON api_keys (tenant_id);

-- Fail closed: a writer that forgets to seal stores nothing (the CHECK refuses it) rather than a
-- plaintext credential at rest.
ALTER TABLE api_keys DROP CONSTRAINT IF EXISTS api_keys_key_sealed_envelope;
ALTER TABLE api_keys ADD CONSTRAINT api_keys_key_sealed_envelope
    CHECK (key_sealed LIKE 'enc:v1:%');
