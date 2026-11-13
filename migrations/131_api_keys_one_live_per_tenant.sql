-- 131 — one LIVE API key per tenant (kanban t_7863696b follow-through).
--
-- The "create answers 409, regenerate is the way to replace" contract rests on this index:
-- a tenant may hold ANY number of rows in api_keys (every regenerate leaves the retired row
-- behind, so the audit trail survives), but at most ONE of them is live at a time.
--
-- `WHERE revoked_at IS NULL` is the liveness predicate for THIS app — never `is_active`.
-- The old retired-credential surface used an `is_active` boolean (migration 000002); this table
-- was rebuilt by migration 130 with `revoked_at` only, and every resolver, the MCP key_auth and
-- this index all agree on `revoked_at IS NULL`.
CREATE UNIQUE INDEX IF NOT EXISTS idx_api_keys_tenant_live_unique
    ON api_keys (tenant_id)
    WHERE revoked_at IS NULL;
