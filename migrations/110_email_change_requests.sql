-- A customer can change their own sign-in address (2026-10-18, cron `default`).
--
-- The profile screen used to say "Your sign-in address cannot be changed here" and nothing
-- anywhere in the app could change `users.email`. Signup collects NAME + EMAIL only, so a
-- single typo at signup left an account that could never receive another message from us.
--
-- One pending change per user: the request handler retires the previous row before inserting,
-- and the confirmation token is a v4 UUID (122 bits, guessed nowhere).
--
-- The table is additive and idempotent: an install that already has it (a replayed boot) is a
-- no-op, and nothing existing is touched.
CREATE TABLE IF NOT EXISTS email_changes (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    new_email   TEXT NOT NULL,
    token       TEXT NOT NULL UNIQUE,
    expires_at  TIMESTAMPTZ NOT NULL,
    used        BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS email_changes_user_idx
    ON email_changes (user_id);

-- The confirmation lookup is `WHERE token = $1 AND used = false AND expires_at > NOW()`.
CREATE INDEX IF NOT EXISTS email_changes_live_idx
    ON email_changes (token) WHERE used = FALSE;
