-- Migration 074: seed the SHIPPED default 'welcome' email template, and normalise the two
-- fresh-build-only rows in the affiliate catalogue (kanban t_6c44acb9, measured 2026-10-02).
--
-- WHY THIS FILE EXISTS
--
-- A from-zero build applies the same 61 migrations but comes up with a DIFFERENT row set from live.
-- Measured on a from-zero database built by booting the shipped artifact against an empty database
-- (61/61 migrations, ledger version 73) vs the live funnelswift database:
--
--   table                fresh   live   verdict
--   email_templates          0      1   SHIPPED product row - seeded here (part 1)
--   affiliate_products       8      7   shipped catalogue - normalised here (parts 2 and 3)
--   admin_settings           0      2   operator - provider credentials + site settings, NOT seeded
--   plans                    4      6   operator - pricing tiers, NOT seeded
--   tags / tag_groups / tag_rules  31 6 1 / 31 6 1  already at parity since 060, nothing to do
--   tenants / users / leads / kinetic_cards  1 0 0 0 / 16 15 56 7  customer data, never seeded
--
-- PART 1 - email_templates. Live carries exactly one row, the global default welcome template
-- (template_type 'welcome', aid NULL, is_default true). It is a SHIPPED product row, not operator
-- state: src/handlers/email_template_handler.rs lists this table as the admin panel's Email
-- Templates screen, so on a fresh install that screen is EMPTY and the operator has nothing to see
-- or edit. The sender (src/email.rs get_db_template -> render_template) does not fail without it -
-- it falls back to get_inline() - but the fallback is a DIFFERENT message: text-only with no
-- html_body and a different subject (measured: the shipped row is
-- 'Welcome to {{app_name}}!' with a 243-byte HTML body, the inline arm is
-- 'Welcome to FunnelSwift!' with html_body = None). So a fresh install sends a welcome mail its
-- own panel cannot show and its own sender would not have chosen. Subject, body and html_body are
-- the LIVE bytes, dollar-quoted so no escaping step can alter them.
--
-- PART 2 - affiliate_products, the stale duplicate. 043_seed_free_affiliate_products.sql seeded the
-- per-app Free products with source_app 'missedcall' for the MissedCall Respondr row and 055's own
-- header records that "043 spelled the last one 'missedcall', which no caller ever sends". 055
-- corrected it by INSERTing the correctly-keyed row (source_app 'missedcallrespondr') and its
-- idempotency guard is (source_app, is_active) - so it never touched 043's row. On live 043's rows
-- had already been destroyed by the System-tenant ON DELETE CASCADE (055's header), so live holds
-- exactly one MissedCall product. A from-zero build holds BOTH, and their system_tag_id is the SAME
-- tag ('MissedCall Respondr - Free'), so:
--   * src/tag_logic.rs attribute_affiliate_on_tags resolves
--     SELECT id FROM affiliate_products WHERE system_tag_id = ANY($1) AND is_active = true
--     and writes ONE commission per returned product, so a fresh install credits TWO products for
--     one lead - a double attribution that live cannot produce, and
--   * affiliate_product_handler::list_affiliate_products lists every row owned by the caller OR the
--     System tenant, so the admin catalogue shows the same product twice.
-- Retiring the stale row makes a fresh build match live. Guarded three ways: only the never-sent
-- key, only when the corrected row is present, and never when a commission points at it (the FK is
-- ON DELETE SET NULL, so the guard is what stops a real attribution being nulled).
--
-- PART 3 - affiliate_products, the illegal plan link only a from-zero build can carry. 043 linked
-- its two FunnelSwift products to plan ids f...001 and f...002. On live those ARE FunnelSwift's own
-- free tiers (live's plans are named Capture Free / Kinetic Free, both price 0), but on a from-zero
-- build they are the generic tiers 000001_initial.sql seeds - f...002 is 'Starter' at price 29.
-- 070's trigger enforce_affiliate_products_are_free() refuses a product linked to a PAID plan and
-- fires BEFORE INSERT OR UPDATE OF plan_id, system_tag_id, name, so on a fresh install that row is
-- a state the product's OWN rule forbids and no admin screen can save. Measured: the first draft of
-- this file renamed the row and the trigger answered
--   ERROR: affiliate product "FunnelSwift Kinetic Free" may not be linked to a PAID plan (price 29)
-- Live cannot reach that state at all, because its plan 002 is free. The unlink below is guarded on
-- a NON-ZERO plan price, so it is a no-op wherever the plan is free (live: both FunnelSwift rows)
-- and only fires on a database whose plan really is paid. 070 explicitly allows a product with no
-- plan link, and routing is unaffected: tag_logic attributes by system_tag_id, the upgrade-event
-- reader by source_app.
--
-- PART 4 - deliberately NOT done: the product NAMES. Live reads 'FunnelSwift Capture Free' /
-- 'FunnelSwift Kinetic Free', a fresh build reads 'Capture Free' / 'Kinetic Free'. That difference
-- is a function of the operator's plan names - 067 mints the name as 'FunnelSwift ' || plan.name and
-- on live the plans are named Capture Free / Kinetic Free - so a constant name here would invent a
-- value the product does not own. Nothing reads either literal (the only two hits in src/ are
-- comments). Reported, not seeded.
--
-- NO-OP ON LIVE, every statement: the welcome row exists (0 inserted), there is no 'missedcall' row
-- (0 deleted), and both FunnelSwift products sit on price-0 plans (0 unlinked). Measured by
-- dry-running this file twice inside a rolled-back transaction against live - see the card's audit
-- directory.
--
-- NOT SEEDED, deliberately (measured, see the header table): admin_settings (the operator's email
-- provider credentials and the funnelswift_site settings object), plans (pricing tiers - live's own
-- six carry capture-free/kinetic-free/... slugs and prices entered in the panel), and every
-- customer-owned row set (tenants, users, leads, kinetic_cards).
--
-- NO SEMICOLONS IN THIS HEADER (the deploy staging path splits on the statement separator, as 050,
-- 0059 and 060 record). The dollar-quoted template bodies below contain none either, so no splitter
-- can cut a statement in half.

-- ── part 1: the shipped default welcome template ────────────────────────────────────────────────
INSERT INTO email_templates (template_type, name, subject, body, html_body, is_default)
SELECT v.template_type, v.name, v.subject, v.body, v.html_body, true
  FROM (VALUES (
    'welcome'::text,
    'Default Welcome Email'::text,
    $fs_welcome_subject$Welcome to {{app_name}}!$fs_welcome_subject$::text,
    $fs_welcome_body$Welcome to {{app_name}}, {{name}}!

Your account has been created successfully.

Your Login Credentials:
Email: {{email}}

Login: {{login_url}}

Best,
The {{app_name}} Team
$fs_welcome_body$::text,
    $fs_welcome_html$<h2>Welcome to {{app_name}}, {{name}}!</h2><p>Your account has been created successfully.</p><p><strong>Your Login Credentials:</strong><br>Email: {{email}}</p><p><a href="{{login_url}}">Log in here</a></p><p>Best,<br>The {{app_name}} Team</p>$fs_welcome_html$::text
  )) AS v(template_type, name, subject, body, html_body)
 WHERE NOT EXISTS (
        SELECT 1 FROM email_templates
         WHERE template_type = 'welcome' AND aid IS NULL AND is_default = true);

-- ── part 2: retire 043's stale 'missedcall' product (055 documents the key as never sent) ───────
DELETE FROM affiliate_products ap
 WHERE ap.source_app = 'missedcall'
   AND EXISTS (SELECT 1 FROM affiliate_products p2
                WHERE p2.source_app = 'missedcallrespondr' AND p2.is_active)
   AND NOT EXISTS (SELECT 1 FROM affiliate_commissions c WHERE c.product_id = ap.id);

-- ── part 3: unlink a FunnelSwift product from a plan the 070 rule forbids (paid) ────────────────
UPDATE affiliate_products ap
   SET plan_id = NULL, updated_at = NOW()
 WHERE ap.source_app = 'funnelswift'
   AND ap.plan_id IS NOT NULL
   AND EXISTS (SELECT 1 FROM plans pl WHERE pl.id = ap.plan_id AND pl.price <> 0);
