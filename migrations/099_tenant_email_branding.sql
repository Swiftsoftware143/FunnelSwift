-- 099_tenant_email_branding.sql
-- David 2026-10-08 (kanban t_c06a32eb): every transactional email this app sends should carry the
-- TENANT's own branding — a logo and a brand display name — settable by the tenant in their own
-- console, so the mail a business's users receive looks like that business's mail.
--
-- Two pieces of storage, and only ONE of them is new:
--
--  1. `tenant_settings` key `email_branding` = {"brand_name", "brand_color", "logo_url"}. No DDL:
--     `tenant_settings` is the tenant's own key/value store (UNIQUE (tenant_id, key), read and
--     written by GET/PUT /api/v1/settings — the same route the tenant Settings screen already
--     uses), and a jsonb document is exactly the shape `email_config` already has there. A tenant
--     with no row is simply unbranded and gets byte-identical mail to before this feature.
--
--  2. `tenant_logos` — the logo's BYTES. Same storage decision as `user_avatars` (migration 098):
--     `docker inspect funnelswift` binds exactly two paths (the release binary and `migrations/`),
--     so a file written at run time lives inside the container and dies with the next
--     `docker restart`, and no host webroot can serve it. The bytes are kept in the database and
--     streamed back by `GET /api/v1/branding/logo/:tenant_id`.
--
-- One row per tenant: the logo is the tenant's, not per-user and not per-app, and the upload path
-- upserts it. Deleting the tenant must take the logo with it, which the FK's ON DELETE CASCADE
-- covers; the primary key is the only lookup path, so no extra index is needed.
CREATE TABLE IF NOT EXISTS tenant_logos (
    tenant_id    uuid         PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    content_type varchar(100) NOT NULL,
    bytes        bytea        NOT NULL,
    updated_at   timestamp    NOT NULL DEFAULT NOW()
);
