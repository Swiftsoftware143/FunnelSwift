-- 102_mobile_platform_interest.sql
-- FunnelSwift's /download-app page promised the visitor something it could not do. The iOS box read
-- "Leave your email to get notified when it launches" and then offered
-- `<a class="dl-btn gray">Coming Soon — Notify Me</a>` — an anchor with NO href, no input field and
-- no backend (measured 2026-10-16 on the served page, which the homepage nav links to twice and
-- /kinetic links to once: a real click did nothing).
--
-- This table is where that promise is kept. One row per (platform, email):
--
--   * `platform` — which app store the interest is in. Android is already shipped (the APK on the
--     same page downloads), iOS is what the capture is FOR.
--   * `email`    — normalised (trimmed, lower-cased) by the handler so the UNIQUE guard below
--     actually merges what a person would consider one address.
--   * `source`   — which page/form captured it, so a second capture point can be told apart later.
--
-- NO tenant column, deliberately: this interest is in the PRODUCT's own mobile app, not in any
-- customer's workspace, so it belongs to no tenant and must never appear in a tenant-scoped query.
--
-- The UNIQUE constraint IS the idempotency guard: the handler inserts ON CONFLICT DO NOTHING, so a
-- double click, a refresh or a repeated POST records nothing new and still answers the same
-- confirmation — a visitor is never shown a duplicate error for something that worked.
CREATE TABLE IF NOT EXISTS mobile_platform_interest (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    platform   varchar(32) NOT NULL,
    email      varchar(254) NOT NULL,
    source     varchar(64) NOT NULL DEFAULT 'download-app',
    created_at timestamp NOT NULL DEFAULT NOW(),
    CONSTRAINT mobile_platform_interest_platform_email_key UNIQUE (platform, email)
);

-- The only read the operator's list does: newest first, within one platform.
CREATE INDEX IF NOT EXISTS mobile_platform_interest_platform_created_idx
    ON mobile_platform_interest (platform, created_at DESC);
