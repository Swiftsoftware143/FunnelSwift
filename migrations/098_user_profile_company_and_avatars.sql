-- 098_user_profile_company_and_avatars.sql
-- David 2026-10-08 (kanban t_ff948669): the end-user console has no profile screen, so a customer
-- cannot set their own name/company or change their password. Two pieces of storage were missing:
--
--  1. `users.company`. The signup form collects NAME + EMAIL only, so nothing in the product has
--     ever written a company. The profile screen asks for one, so the column has to exist.
--
--  2. `user_avatars`. `users.avatar_url` already existed but there was no way to fill it. The
--     fleet's usual upload shape (IncentiveSwift's `upload_file`) writes the file into a served
--     webroot and stores the URL; THIS app cannot: `docker inspect funnelswift` binds exactly two
--     paths (the release binary and `migrations/`), so a file written at run time is inside the
--     container and dies with the next `docker restart` — and no host webroot can serve it. The
--     bytes are therefore kept in the database and streamed back by `GET /api/v1/auth/avatar/:id`,
--     which is what makes the picture survive a deploy.
--
-- The image is deliberately NOT a column on `users`: `change_password` and every other reader does
-- `SELECT * FROM users`, and a multi-megabyte bytea on that row would be fetched on every one of
-- those reads.
ALTER TABLE users ADD COLUMN IF NOT EXISTS company varchar(255);

CREATE TABLE IF NOT EXISTS user_avatars (
    user_id      uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    content_type varchar(100) NOT NULL,
    bytes        bytea NOT NULL,
    updated_at   timestamp NOT NULL DEFAULT NOW()
);

-- Deleting a user must not leave the picture behind; the FK's ON DELETE CASCADE covers that, and
-- this index is the only lookup path (one row per user, by primary key, so no extra index is needed).
