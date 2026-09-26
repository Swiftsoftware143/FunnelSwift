-- Migration 032: the objects the live database has that NO migration in this chain creates.
--
-- WHY THIS FILE EXISTS (kanban t_f7341aa9, measured 2026-09-26)
--   Fixing the kinetic_cards ordering defect (file 0004) let the chain past 0015, and the next
--   from-zero run died at 043 with
--       column "tenant_id" of relation "affiliate_products" does not exist
--   A file-by-file census of the whole directory on a fresh database (one transaction per file,
--   ON_ERROR_STOP = 1) measured the complete blocker set, not just the first one:
--       FAIL 043_seed_free_affiliate_products.sql :: column "tenant_id" of relation "affiliate_products" does not exist
--       FAIL 055_affiliate_product_source_app_keys.sql :: column "tenant_id" of relation "affiliate_products" does not exist
--       FAIL 056_workspace_delete_owns_its_rows.sql :: relation "campaigns" does not exist
--   Root cause, measured against the live database: production was built out-of-band, so it
--   carries THREE tables and 36 columns that no migration declares -- affiliate_products.tenant_id
--   (which 043 and 055 insert into), campaigns (which 056 casts to regclass), the rest silently
--   missing. A fresh database therefore had a different schema than production in BOTH directions
--   (62 columns and 3 tables absent, plus 4 columns declared with a different type and 6 indexes
--   the chain never creates), which is what made every from-zero environment a false oracle.
--
-- WHAT THIS FILE DOES
--   Emits, verbatim from the live catalog (pg_attribute + pg_attrdef + pg_indexes), the objects
--   the chain is missing: the 3 tables, their 36 columns, the 6 indexes, the 4 type alignments,
--   and the one constraint the live database validates by hand (050 documents the post-backfill
--   VALIDATE step, and live is VALID while a fresh build was NOT VALID).
--   Every statement is guarded (IF NOT EXISTS / type-diff test / NOT convalidated), so on any
--   database that already has the object this file is a NO-OP -- which is what it is on live.
--
-- WHY VERSION 32
--   The blockers need affiliate_products.tenant_id before 043 and campaigns before 056, so the
--   file must sort before 043. Versions 32 through 41 are vacant in this directory and a gap is
--   legal (sqlx applies every version not yet in _sqlx_migrations, in numeric order -- see
--   /opt/swift/fleet/migration-version-check.sh).
--
-- WHY NOT EDIT 043 / 055 / 056
--   All three are recorded in _sqlx_migrations on every live database, and sqlx aborts the WHOLE
--   run with VersionMismatch when an APPLIED file's checksum changes -- editing them would stop
--   production from ever applying another migration. New files are the only repair that reaches a
--   fresh build and leaves live alone.
--
-- NO SEMICOLONS IN THIS HEADER, deliberately: 050_provider_keys_encrypted_at_rest.sql records
--   that the deploy staging path splits a migration file on the statement separator.

CREATE TABLE IF NOT EXISTS admin_settings (
    key text NOT NULL PRIMARY KEY,
    value jsonb NOT NULL DEFAULT '{}'::jsonb,
    description text,
    updated_at timestamp with time zone NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS campaigns (
    id character varying(64) NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name character varying(255) NOT NULL,
    campaign_type character varying(64),
    description text,
    status character varying(32) DEFAULT 'draft'::character varying,
    created_at timestamp without time zone DEFAULT now(),
    updated_at timestamp without time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS email_templates (
    id uuid NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    template_type text NOT NULL,
    name text NOT NULL,
    subject text NOT NULL,
    body text,
    html_body text,
    is_default boolean DEFAULT false,
    aid uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

-- affiliate_products
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS tenant_id uuid;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS updated_at timestamp with time zone DEFAULT now();
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS default_commission_rate numeric(5,2) DEFAULT 10.0;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS is_third_party boolean DEFAULT false;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS slug character varying(255);
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS referral_code character varying(50);
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS affiliate_id uuid;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS image_url text;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS website_url text;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS integration_type character varying(50);
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS url text;
ALTER TABLE affiliate_products ADD COLUMN IF NOT EXISTS category_id uuid;

-- affiliates
ALTER TABLE affiliates ADD COLUMN IF NOT EXISTS is_visible boolean DEFAULT true;
ALTER TABLE affiliates ADD COLUMN IF NOT EXISTS tags jsonb DEFAULT '[]'::jsonb;

-- kinetic_card_events
ALTER TABLE kinetic_card_events ADD COLUMN IF NOT EXISTS click_label text;
ALTER TABLE kinetic_card_events ADD COLUMN IF NOT EXISTS click_url text;
ALTER TABLE kinetic_card_events ADD COLUMN IF NOT EXISTS ip_address text;
ALTER TABLE kinetic_card_events ADD COLUMN IF NOT EXISTS screen_size text;

-- plans
ALTER TABLE plans ADD COLUMN IF NOT EXISTS annual_price double precision;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS billing_cycle character varying(10) NOT NULL DEFAULT 'month'::character varying;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_analytics boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_api boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_card_gating boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_import_export boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_mini_funnels boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_remove_branding boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS has_webhooks boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_action_buttons integer DEFAULT 3;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_cards integer DEFAULT 1;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_custom_domains integer NOT NULL DEFAULT 0;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_forms integer DEFAULT 1;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_ocr_scans integer DEFAULT 0;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_qr_codes integer DEFAULT 1;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_team_members integer NOT NULL DEFAULT 2;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS side character varying(20) NOT NULL DEFAULT 'main'::character varying;

-- tags
ALTER TABLE tags ADD COLUMN IF NOT EXISTS updated_at timestamp with time zone DEFAULT now();

DO $mig$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = 'leads' AND column_name = 'created_at'
                AND data_type = 'timestamp without time zone') THEN
    ALTER TABLE leads ALTER COLUMN created_at TYPE timestamptz;
  END IF;
END $mig$;

DO $mig$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = 'leads' AND column_name = 'updated_at'
                AND data_type = 'timestamp without time zone') THEN
    ALTER TABLE leads ALTER COLUMN updated_at TYPE timestamptz;
  END IF;
END $mig$;

DO $mig$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = 'tags' AND column_name = 'created_at'
                AND data_type = 'timestamp without time zone') THEN
    ALTER TABLE tags ALTER COLUMN created_at TYPE timestamptz;
  END IF;
END $mig$;

DO $mig$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = 'plans' AND column_name = 'price'
                AND data_type = 'numeric') THEN
    ALTER TABLE plans ALTER COLUMN price TYPE double precision;
  END IF;
END $mig$;

-- the tenant edge live declares on affiliate_products (056's census of missing edges did not
-- include this table because live already had it) -- present in production, absent from the chain
DO $mig$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint
                  WHERE conname = 'affiliate_products_tenant_id_fkey'
                    AND conrelid = 'public.affiliate_products'::regclass) THEN
    ALTER TABLE affiliate_products
      ADD CONSTRAINT affiliate_products_tenant_id_fkey
      FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
  END IF;
END $mig$;

CREATE INDEX IF NOT EXISTS idx_card_locations_card ON public.kinetic_card_locations USING btree (card_id);
CREATE INDEX IF NOT EXISTS idx_card_locations_country ON public.kinetic_card_locations USING btree (country);
CREATE UNIQUE INDEX IF NOT EXISTS idx_email_templates_unique ON public.email_templates USING btree (template_type, COALESCE(aid, '00000000-0000-0000-0000-000000000000'::uuid), is_default) WHERE ((aid IS NULL) AND (is_default = true));

-- (The provider_keys encryption CHECK is validated by 059, not here: 050 creates it NOT VALID and
--  050 sorts AFTER this file, so at this position the constraint does not exist yet.)
