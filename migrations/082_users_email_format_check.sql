-- 082_users_email_format_check.sql
--
-- Store-level backstop for the login-identity column (kanban t_38017305).
--
-- FunnelSwift had NO format check on `users.email` anywhere in the request path, and none on the
-- column either, so the public signup routes stored whatever the caller sent: `bad` on
-- POST /api/v1/auth/register (no guard at all) and anything containing an `@` on
-- POST /api/v1/auth/signup. Either way the account was real and its login was not an address, so no
-- credentials mail could ever reach it. `src/security/email_addr.rs` is the single rule now applied
-- in the request path (trim + lowercase + syntax check); this CHECK mirrors it at the store.
--
-- Deliberately LOOSER than the Rust rule, which additionally rejects `.user@x.com`, `us..er@x.com`,
-- `user@.x.com`, `user@x..com`, `user@x.com.` and anything over 254 characters. The database must
-- never refuse a value the application accepted, so this only requires "local@domain.tld" with no
-- whitespace and no second `@`. Measured before shipping: 0 rows in `users` violate it
-- (`SELECT count(*) FROM users WHERE email !~ '…'` = 0), so ADD CONSTRAINT cannot fail on live data.
ALTER TABLE users
    ADD CONSTRAINT users_email_format_check
    CHECK (email ~ '^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$');
