-- 054_create_funnels_tiers_referral_tracking.sql
-- FunnelSwift: three relations named by plain statements that NO migration ever created
-- (card t_7197ad72, TABLE-MISSING class of fleet-dbtype-audit.py).  One verdict per site,
-- decided from the DECODE side; evidence in /opt/swift/audits/t_7197ad72/REPORT.md.
--
--   funnels            -> CREATE TABLE (this file).  src/handlers/funnel_handler.rs is a full
--       CRUD over it (INSERT :37, SELECT :56, UPDATE :74, DELETE :89, render_funnel :141 reads
--       it by slug for the PUBLIC /funnel/<slug> page) and src/handlers/seo_handler.rs:49
--       builds the /funnel/<slug> sitemap entries from it.  REPOINT is impossible: a
--       whole-database check found no other relation in funnelswift that models a funnel, so
--       the feature has a writer, a reader and no table.
--   affiliate_tiers    -> CREATE TABLE (this file).  src/handlers/affiliate_payout_handler.rs
--       holds the whole CRUD (SELECT :35 and :86, INSERT :59, UPDATE :99, DELETE :120).
--       Column types are the decode side's, read out of sqlx 0.7: commission_rate and
--       min_revenue are decoded as f64, and f64 accepts FLOAT8 only -- a NUMERIC column fails
--       with "mismatched types" (exactly the trap t_4385aa2c fixed in list_payouts with a
--       ::float8 cast), hence DOUBLE PRECISION.  created_at is decoded as chrono::NaiveDateTime,
--       which sqlx maps to `timestamp without time zone` -- so it is TIMESTAMP, not
--       timestamptz (see the same decision in activity_log / affiliates / plans).
--   referral_tracking  -> CREATE TABLE (this file).  src/handlers/public_signup_handler.rs:136
--       INSERTs the affiliate code a signup arrived through.  REPOINT considered and rejected:
--       the neighbouring attribution tables are affiliate_clicks (needs link_id) and
--       affiliate_conversions (a SALE: amount/commission/status) -- neither records "this
--       workspace signed up through code X".  Stated, not hidden: the row is written and
--       nothing in the repository reads it yet (an affiliate-signup report is a separate card);
--       today the INSERT is a 42P01 swallowed by `let _ =`, so the attribution is simply lost.
--
--   qr_codes           -> NO DDL, and none is wanted (this file creates nothing for it).
--       The name was PORTED IN; the app's canonical QR model is `kinetic_qr_codes` (migration
--       0015, "per-card QR generation with plan-based limits"), which is also the table
--       features::count_usage counts for the `max_qr_codes` plan gate.  A second QR table would
--       leave that gate counting a table nothing writes -- a gate that stops erroring and
--       starts LYING -- so qr_handler.rs was repointed at the canonical table instead.
--
-- Idempotent (IF NOT EXISTS): re-runs safely on a fresh, restored or already-fixed database.
-- No BEGIN/COMMIT: sqlx wraps each migration in its own transaction.

CREATE TABLE IF NOT EXISTS funnels (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  UUID NOT NULL,
    name       TEXT NOT NULL,
    slug       TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Published at /funnel/<slug> (slug-only lookup, no tenant), the same shape as
-- kinetic_cards' slug index: a plain index, because a UNIQUE constraint would 500
-- create_funnel on a name collision the writer cannot yet resolve.
CREATE INDEX IF NOT EXISTS idx_funnels_slug ON funnels(slug);
CREATE INDEX IF NOT EXISTS idx_funnels_tenant ON funnels(tenant_id);

CREATE TABLE IF NOT EXISTS affiliate_tiers (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    commission_rate DOUBLE PRECISION NOT NULL DEFAULT 10.0,
    min_sales       INTEGER NOT NULL DEFAULT 0,
    min_revenue     DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    description     TEXT,
    created_at      TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_affiliate_tiers_min_sales ON affiliate_tiers(min_sales);

CREATE TABLE IF NOT EXISTS referral_tracking (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    referrer_code      TEXT NOT NULL,
    referred_email     TEXT NOT NULL,
    referred_tenant_id UUID NOT NULL,
    created_at         TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_referral_tracking_code ON referral_tracking(referrer_code);
CREATE INDEX IF NOT EXISTS idx_referral_tracking_tenant ON referral_tracking(referred_tenant_id);
