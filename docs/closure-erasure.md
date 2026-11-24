# Closure erasure — the 30-day timer

**Fleet standard §2:** *"A closure auto-erases after 30 days unless support reverses it."* This is the
half of that promise that runs by itself. The other half — the customer's **Delete account and data**
button — is the Danger zone in the profile screen, and it is unchanged.

Everything here is about **Cancel plan** (`POST /api/v1/auth/close-account`). That door ends access and
keeps every row on purpose; it does not delete anything on the day. This job is what makes the kept
data temporary.

## What runs, and when

| | |
|---|---|
| Job | `ops/closure_erasure.py` (this repo — the canonical copy) |
| Schedule | root's crontab, once a day at **06:47 off-peak** (`47 6 * * *`, installed 2026-11-24) |
| Log | `/var/log/funnelswift-closure-erasure.log` (one line per workspace, plus a `SUMMARY` JSON line) |
| Purge | `funnelswift_erase_tenant(tenant, NULL, 'closure-30d')` — migration 144 |
| Census | `funnelswift_closures_due(30)` — migration 144, where every rule lives |
| Audit | one `account_erasures` row per erasure, `reason = 'closure-30d'`, `actor_ref` NULL (no human pressed it) |

## What it erases, and what it keeps

Exactly what the Danger zone's erase does, because it is the same SQL function:

* **Deleted:** the workspace's leads, funnels, cards, campaigns, tags, forms, inbox mail and attachments,
  media/profile rows, team members and their sign-ins, sessions, API keys — every row in every table that
  carries the workspace id, then the workspace row itself. The function refuses to report success while
  any row still names the workspace.
* **Kept:** rows in `invoices` — billing documents required by tax law. They are **detached**
  (`tenant_id` → NULL) rather than deleted. The audit row keeps the workspace *name* for those invoices.

## Which workspaces it will never touch

`funnelswift_closures_due()` returns a workspace only when **all** of these hold. Anything else is left
alone and is not even mentioned in the log:

1. It has at least one person (a workspace with no users records no closure — there is nothing to honour).
2. **Every** person in it is closed. If one teammate is still signed in, the workspace is never erased
   under them.
3. No person in it is a platform operator (`admin` / `super_admin`). An install's own console is not a
   customer.
4. The **last** closure is at least **30 days** old. The clock starts when the last person leaves, not
   when the first one did.
5. It is not the system seed, a fleet fixture (`probe-canary-*`), a probe-harness workspace, or a company
   the portfolio registry / name list owns.

The 30 days are a **floor in SQL** (`GREATEST(days, 30)`) — no script argument, cron typo or casual
`SELECT` can shorten the customer's restore window.

## Running it by hand

```bash
# What would be erased right now — changes nothing (the default):
/opt/swift/apps/FunnelSwift/ops/closure_erasure.py

# Erase them now (what the 06:47 cron line does):
/opt/swift/apps/FunnelSwift/ops/closure_erasure.py --apply

# Census only, straight from SQL:
psql "$(grep -m1 '^DATABASE_URL=' /etc/swift/env/funnelswift.env | cut -d= -f2-)" \
     -c "SELECT * FROM funnelswift_closures_due(30)"
```

The job refuses to run against any database whose name is not `funnelswift`, so a pasted sibling-app
URL or a leftover scratch copy stops it instead of erasing the wrong fleet. It exits `3` when it cannot
run at all and `1` when a workspace failed to erase (the others still go); the next run picks up any
failure, and re-running is a no-op for what already went.

## Reversing a closure (support)

A closure is reversible for 30 days. Clearing it makes the workspace invisible to the next census,
because the window is measured from `users.closed_at` at run time:

```sql
-- ONE workspace: reopen everyone in it who is closed.
UPDATE users
   SET closed_at = NULL, updated_at = now()
 WHERE tenant_id = '<tenant id>'
   AND closed_at IS NOT NULL;
```

Then tell the customer:

* they sign in normally at the app's sign-in page (a closed account cannot ask for a password reset, so
  if they do not remember their password, support sets a new one from the operator console first);
* **every API key was revoked at the moment of cancellation** and stays revoked — the customer reissues
  the keys their tools use. Reopening does not restore a revoked key, by design.

Confirm the reversal took by re-running the dry run above: the workspace must not appear.

**After day 30 there is nothing to reverse** — the workspace and its records are gone, and only its
invoices remain (detached, with the workspace name on the audit row). Say that plainly to the customer
instead of promising a restore.

## Finding out what happened

```sql
-- every automatic closure erasure, newest first
SELECT created_at, tenant_name, rows_deleted, invoices_retained
  FROM account_erasures
 WHERE reason LIKE 'closure-%'
 ORDER BY created_at DESC;

-- the erasures a person pressed (the Danger zone), for contrast
SELECT created_at, tenant_name, rows_deleted, invoices_retained
  FROM account_erasures
 WHERE reason = 'owner-request'
 ORDER BY created_at DESC;
```

The workspace's id (the `account_erasures.tenant_ref`, also the tenant id support would have had) is
enough to search the nightly database dumps if a restore is ever ordered for an erased workspace — but
an erase is not reversible in the product, and the customer was told so before pressing.

## Proof that this works

The job, the census and the whole purge were proved on a scratch copy of the live database: one
workspace closed 31 days ago erased (leads gone, invoice kept and detached, one audit row with
`reason = closure-30d`), one closed 29 days ago untouched, a workspace with an active teammate untouched,
and the canary / harness / operator / portfolio cases untouched; a cleared closure immediately dropped
out of the census and came back when it was set again; a second `--apply` erased nothing. The recipe is
at the top of `ops/closure_erasure.py`. Never run the proof against the live database.
