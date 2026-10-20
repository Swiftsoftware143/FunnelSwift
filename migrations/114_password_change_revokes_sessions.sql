-- A password change or reset must end the sessions that predate it.
--
-- Measured 2026-10-20 (kanban t_80787a0f): on a live probe user, a 30-day JWT minted BEFORE a
-- password reset still answered 200 from `GET /api/v1/auth/me` after the reset, and a JWT minted
-- before a password CHANGE survived the change the same way. The token lives 30 days, so the one
-- action a user takes when they believe their session is compromised — changing the password —
-- revoked nothing, and the attacker's session outlived it.
--
-- NULL means "never restricted": every existing account and every token already in the wild keeps
-- working exactly as before, so this is additive. The column only takes a value when a password is
-- changed or reset, and `auth::global_auth` refuses any JWT whose `iat` is older than it.
ALTER TABLE users ADD COLUMN IF NOT EXISTS sessions_valid_from timestamptz;

COMMENT ON COLUMN users.sessions_valid_from IS
    'JWTs issued before this instant are refused (revocation on password change/reset); NULL = never restricted.';
