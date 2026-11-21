-- 138_portfolio_company_account.sql — a portfolio company's OWN account, kept apart from the
-- owner's (kanban t_4a9454c1, 2026-11-21, fleet requirement of 2026-11-20).
--
-- WHY. `portfolio_companies.tenant_id` carries ONE account reference and it means different things
-- depending on which door wrote the row:
--   * the cross-app sync doors (`admin_handler::portfolio_sync`,
--     `portfolio_sync_handler::*`) store the COMPANY'S OWN tenant there;
--   * the user-facing `POST /api/v1/portfolio-companies` stores the CALLER'S tenant — the owner's
--     account, the same one every portfolio row shares.
-- So "open the row's tenant" enters the operator's own account for a row written by the second
-- door: a button that looks like it does nothing. A portfolio row needs BOTH references, said out
-- loud:
--   tenant_id         = the account that OWNS the row (operator / caller scope)   [unchanged]
--   account_tenant_id = the COMPANY'S OWN account, NULL when it does not have one yet
--
-- The customer list (`GET /api/v1/tenants`) hides every tenant whose id appears here, so the
-- owner's own businesses never sit among paying customers again. The Portfolio screen opens a row
-- with THIS column, never `tenant_id`.
--
-- BACKFILL is exact-match only (tenant slug = company slug; `tenants_slug_key` makes the slug
-- unique, so the match is unambiguous) and refuses any tenant another portfolio row already
-- claims. It can never point a company at the owner's own account, and it is idempotent: it only
-- fills rows where account_tenant_id IS NULL (this runner re-executes every file at boot).
ALTER TABLE portfolio_companies
    ADD COLUMN IF NOT EXISTS account_tenant_id uuid REFERENCES tenants(id) ON DELETE SET NULL;

UPDATE portfolio_companies pc
   SET account_tenant_id = t.id
  FROM tenants t
 WHERE pc.account_tenant_id IS NULL
   AND lower(t.slug) = lower(pc.slug)
   AND NOT EXISTS (
        SELECT 1 FROM portfolio_companies own
         WHERE own.account_tenant_id = t.id
           AND own.id <> pc.id);

CREATE INDEX IF NOT EXISTS idx_portfolio_companies_account_tenant
    ON portfolio_companies(account_tenant_id);

COMMENT ON COLUMN portfolio_companies.account_tenant_id IS
  'The portfolio company''s OWN account (tenants.id). NULL = the company has no account in this app yet. Never the owner''s account; see kanban t_4a9454c1.';
