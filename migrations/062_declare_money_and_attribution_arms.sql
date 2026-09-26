-- Migration 062: a declared arm for the two money pointers that could still be silently erased,
-- and a recorded WHY-NOT for the two pointers that must not get one.
--
-- Card: kanban t_4dd9e87b §3 ("three ownership pointers are still undeclared and have no domain arm
-- -- all 0 rows today, so this is a decision, not a migration"). Same sweep that produced migration
-- 056 (t_3bfda0b2), which declared every workspace-owned pointer and left these four named.
--
-- MEASURED on the live `funnelswift` database, 2026-09-26: all four tables are EMPTY
--   affiliate_conversions 0   affiliate_payouts 0   referral_tracking 0   contact_tags 0
-- and none of the four columns carries a foreign key (pg_constraint, contype='f' -> 0 rows).
-- So nothing here is a repair; it is the schema stating an ownership rule before the first row
-- makes it expensive to change. Each arm is decided from the READER, not from the column name.

-- ── 1) DECLARED: the money pointers, ON DELETE SET NULL ────────────────────────────────────────
-- affiliate_conversions.affiliate_user_id -> users(id)   (nullable)
-- affiliate_payouts.affiliate_user_id     -> users(id)   (nullable)
--   The stored value is a users.id, measured from the readers, not guessed:
--     src/handlers/affiliate_tracking_handler.rs:68  JOIN affiliate_users au ON au.user_id = ac.affiliate_user_id
--     src/handlers/affiliate_payout_handler.rs:149   LEFT JOIN affiliates a ON a.user_id = p.affiliate_user_id
--   The arm is SET NULL and never CASCADE, because these are MONEY records: a workspace DELETE
--   cascades users (users.tenant_id -> tenants ON DELETE CASCADE, migration 056), so CASCADE here
--   would erase an affiliate's recorded earnings and approved payouts as a side effect of a
--   workspace leaving. RESTRICT/NO ACTION is the other wrong answer: it would re-create exactly the
--   500 "Database error" blocker class 056 had to remove (provider_keys.tenant_id,
--   password_resets.user_id). SET NULL keeps the ledger row and clears ONLY the ownership pointer.
--   Note the deliberate asymmetry with the click edge below, which 056 made CASCADE: the child is
--   judged by what its own row is worth, not by which table it hangs from.
DO $mig$
DECLARE c record;
BEGIN
  FOR c IN
    SELECT * FROM (VALUES
      ('affiliate_conversions', 'affiliate_user_id', 'users'),
      ('affiliate_payouts',     'affiliate_user_id', 'users')
    ) AS v(tbl, col, parent)
  LOOP
    IF NOT EXISTS (
      SELECT 1 FROM pg_constraint
       WHERE conname = c.tbl || '_' || c.col || '_fkey'
         AND conrelid = ('public.' || quote_ident(c.tbl))::regclass)
    THEN
      EXECUTE format(
        'ALTER TABLE %I ADD CONSTRAINT %I FOREIGN KEY (%I) REFERENCES %I(id) ON DELETE SET NULL',
        c.tbl, c.tbl || '_' || c.col || '_fkey', c.col, c.parent);
      RAISE NOTICE '062: declared %.% -> %(id) ON DELETE SET NULL (money record: survives its owner)',
                   c.tbl, c.col, c.parent;
    END IF;
  END LOOP;

  -- affiliate_conversions.click_id -> affiliate_clicks(id), also SET NULL. A click is disposable
  -- traffic (its own child affiliate_clicks.link_id is CASCADE in 056, because a click without its
  -- link is unreadable); a conversion is a commission. Deleting a click must not erase the
  -- commission, so the arms differ on purpose.
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
     WHERE conname = 'affiliate_conversions_click_id_fkey'
       AND conrelid = 'public.affiliate_conversions'::regclass)
  THEN
    ALTER TABLE affiliate_conversions
      ADD CONSTRAINT affiliate_conversions_click_id_fkey
      FOREIGN KEY (click_id) REFERENCES affiliate_clicks(id) ON DELETE SET NULL;
    RAISE NOTICE '062: declared affiliate_conversions.click_id -> affiliate_clicks(id) ON DELETE SET NULL (commission survives its click)';
  END IF;
END $mig$;

-- ── 2) WHY-NOT: referral_tracking.referred_tenant_id — no arm is possible today ────────────────
-- referral_tracking records "this workspace signed up through affiliate code X" (written by
-- src/handlers/public_signup_handler.rs:183). `referred_tenant_id` is the REFERRED workspace; the
-- attribution belongs to the REFERRER, so the two candidate arms are both wrong:
--   CASCADE  -> tenant B deleting itself erases tenant A's referral credit (tenant A loses money).
--   RESTRICT / NO ACTION -> the referred workspace could not be deleted at all (the 23503/500 class
--                migration 056 exists to remove).
--   SET NULL -> impossible without a schema change: the column is NOT NULL, and the reader decodes
--                it as a non-Option value (src/handlers/affiliate_referral_handler.rs:136
--                `let referred_tenant_id: Uuid = r.try_get("referred_tenant_id")?;`), so a NULL
--                would turn a live report into a decode error instead of a missing name. The same
--                reader already tolerates the workspace being gone (it resolves the display name
--                with a sub-select, :55), which is what a dangling id looks like on screen.
-- Conclusion: deliberately LEFT UNDECLARED. The column is an attribution stamp, not an ownership
-- pointer, and the honest place for "settle the credit before the workspace goes" is the delete
-- path, not a constraint that would either destroy or block. Recorded here so the next sweep does
-- not re-open it as an oversight. 0 rows today — the moment the affiliate signup report ships, the
-- right follow-up is a card on the DELETE path, not an FK.

-- ── 3) WHY-NOT: contact_tags.contact_id — there is no parent to point at ───────────────────────
-- `contacts` does not exist in this database (information_schema.tables: 0 rows). The name is
-- ported in from the sibling apps that DO have a contacts table (coreswift_crm, incentiveswift,
-- missedcallrespondr each have one) — a cross-app table name in a schema that never had that table.
-- The only FK on the table is contact_tags.tag_id -> tags(id) ON DELETE CASCADE (migration 056).
-- Nothing in the repository reads or writes contact_tags (grep over src/, www/, www-admin/: 0 hits)
-- and the table holds 0 rows, so there is no writer whose intent could be measured and no parent
-- whose deletion could orphan it. Inventing a parent (leads? whose `contact`-shaped columns are
-- first_name/last_name/email) would be guessing the semantics of a column nothing uses.
-- Conclusion: deliberately LEFT UNDECLARED, with this reason recorded, until a `contacts` table or
-- a writer exists — at which point the arm is decided from that reader.
