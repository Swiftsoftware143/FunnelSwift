-- 145: give every workspace row the address of its own owner (measured defect, 2026-11-25).
--
-- WHY. `tenants.email` (migration 052) is the workspace's contact address. The console's create
-- path has always written it, but the three SELF-SERVICE doors (public signup, /auth/register, the
-- admin mint) created the tenant row without it and put the address only on `users.email`. Measured
-- on LIVE: every signup-created workspace carried `tenants.email = NULL` — including a real
-- customer (`EMI Mold Removal Tulsa OK`) — while every console-created row carried its address. The
-- operator's Tenants list could only fall back to a per-row `users` subquery, and the code that
-- uses the workspace row itself ("resend the owner's access", portfolio sync) had no address.
--
-- WHAT. Fill it, and only it: rows where the column is empty get the address of the person the list
-- already treats as the owner (an admin/company_admin row first, then the oldest login — the SAME
-- ordering `tenant_handler::list_tenants` uses, so the column and the list can never disagree).
-- An existing value is never overwritten. Probe/harness workspaces are skipped: their addresses are
-- deliberately undeliverable and must not end up in the operator's contact column.
--
-- Idempotent: a second run finds nothing empty and updates zero rows.
UPDATE tenants t
SET email = sub.email
FROM (
    SELECT DISTINCT ON (u.tenant_id)
           u.tenant_id,
           u.email
      FROM users u
     WHERE u.email IS NOT NULL
       AND btrim(u.email) <> ''
     ORDER BY u.tenant_id,
              CASE WHEN u.role IN ('admin','company_admin') THEN 0 ELSE 1 END,
              u.created_at ASC
) sub
WHERE t.id = sub.tenant_id
  AND t.probe_harness IS NULL
  AND (t.email IS NULL OR btrim(t.email) = '');
