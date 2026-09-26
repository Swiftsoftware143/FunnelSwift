-- FunnelSwift kanban t_8101edb5 — mint `tenants.affiliate_code` (nothing ever wrote it).
--
-- THE GAP (measured 2026-09-25): `tenants.affiliate_code` is the ONLY namespace in this schema that
-- produces a user-visible `?ref=` link — `kinetic_handler` renders
-- `https://funnelswift.net/kinetic?ref={code}` into the card branding badge, `www/kinetic.html`
-- reads that `?ref=` back into the signup body, and `affiliate_referral_handler` publishes
-- `https://funnelswift.net/signup?ref={code}` as the tenant's share link. Nothing in `src/` ever
-- WROTE the column (only reads), so 126 of 127 live tenants had NULL and no shipped surface could
-- hand a tenant the code it is told to share.
--
-- WHY A TRIGGER AND NOT FIVE MORE INSERT STATEMENTS: tenants are created in five places
-- (`auth/handlers.rs`, `admin_handler.rs`, `tenant_handler.rs`, `public_signup_handler.rs`,
-- `portfolio_sync_handler.rs`). A `BEFORE INSERT` trigger makes "every tenant has a shareable code"
-- a property of the TABLE, so a sixth creation path added later cannot silently reintroduce the
-- defect. The code is a pure function of the row (slug + id), so the backfill and the trigger
-- cannot drift apart:
--
--     <slug, lowercased/alnum, 24 chars>-<first 8 hex of md5(id)>
--
-- Uniqueness needs no retry loop: `tenants.slug` is UNIQUE, and two rows that share a slug cannot
-- exist; different ids give different suffixes. An existing non-empty code is NEVER overwritten
-- (the live `SwiftSoftware` -> `kinetic-87586a09` was set out-of-band and is left exactly as-is).
--
-- Runner: `sqlx::migrate!` (applies once, checksummed). Never edit this file after it ships.

-- Pure, IMMUTABLE so it can be called from the trigger, the backfill and ad-hoc proofs alike.
CREATE OR REPLACE FUNCTION funnelswift_affiliate_code(p_slug text, p_name text, p_id uuid)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $$
  SELECT coalesce(
           nullif(
             btrim(
               left(
                 btrim(
                   regexp_replace(
                     lower(coalesce(nullif(btrim(p_slug), ''), nullif(btrim(p_name), ''), 'tenant')),
                     '[^a-z0-9]+', '-', 'g'
                   ),
                   '-'
                 ),
                 24
               ),
               '-'
             ),
             ''),
           'tenant')
         || '-' || substr(md5(p_id::text), 1, 8)
$$;

CREATE OR REPLACE FUNCTION funnelswift_tenants_mint_affiliate_code()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF NEW.affiliate_code IS NULL OR btrim(NEW.affiliate_code) = '' THEN
    NEW.affiliate_code := funnelswift_affiliate_code(NEW.slug, NEW.name, NEW.id);
  END IF;
  RETURN NEW;
END
$$;

DROP TRIGGER IF EXISTS tenants_mint_affiliate_code ON tenants;
CREATE TRIGGER tenants_mint_affiliate_code
  BEFORE INSERT ON tenants
  FOR EACH ROW
  EXECUTE FUNCTION funnelswift_tenants_mint_affiliate_code();

-- Backfill every pre-existing code-less tenant. Idempotent: rows that already carry a code are not
-- touched (re-running this statement is a no-op), so a restore/replay cannot rename a live code.
UPDATE tenants
   SET affiliate_code = funnelswift_affiliate_code(slug, name, id),
       updated_at     = now()
 WHERE affiliate_code IS NULL
    OR btrim(affiliate_code) = '';
