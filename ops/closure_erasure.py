#!/usr/bin/env python3
"""FunnelSwift closure erasure — the TIMER half of the fleet's account-closure standard (§2).

WHAT IT DOES
    Cancelling (POST /api/v1/auth/close-account) records `users.closed_at` and ends access. The
    standard promises that a closure "auto-erases after 30 days unless support reverses it", so this
    job reads `funnelswift_closures_due(30)` — the census, where every rule lives — and calls the one
    purge, `funnelswift_erase_tenant(tenant, NULL, 'closure-30d')`, once per workspace.

    What the purge deletes and what it keeps is NOT decided here: it is migration 144 / 141. Invoices
    are detached and kept (tax law); every other tenant row goes; the function refuses to report
    success while a row still names the workspace.

    The census skips, by rule, four kinds of workspace: a platform operator's, the system seed and
    fleet fixtures (`probe-canary-*`, probe-harness rows), anything the portfolio registry owns, and
    any workspace where a person is still signed in (every user must be closed, and the 30 days are
    counted from the LAST closure).

WHY A SCRIPT AND NOT A ROUTE
    The standard forbids any API key, internal route or MCP tool from destroying a workspace. A
    scheduled host job holding the database credential is outside every credential boundary the app
    serves, which is exactly the property this needs.

USAGE
    closure_erasure.py                  # DRY RUN (the default): list what would be erased, change nothing
    closure_erasure.py --apply          # erase every workspace the census returns
    closure_erasure.py --days 30        # only ever >= 30 (the SQL floors it; this warns)
    closure_erasure.py --limit 100      # per-run cap, 1..500

    Exit codes: 0 nothing failed, 1 at least one workspace failed to erase (the rest still went),
    3 the job could not run at all (no credential / wrong database / census failed).

SAFETY RAILS (each one is a real incident avoided)
    * Dry run by default: a bare invocation, a manual poke or a typo can never destroy anything.
    * `current_database()` must be funnelswift: a pasted DATABASE_URL from a sibling app, or a
      scratch copy left in the environment, stops the run instead of erasing the wrong fleet.
      (Override with --expected-db ONLY for a proof on a scratch copy.)
    * The 30-day floor lives in SQL, so no argument can shorten the customer's restore window.
    * One statement per workspace: a failure is logged and the run continues; the next run picks it up.
    * Idempotent by construction: an erased workspace is gone from the census, so a re-run finds
      nothing. A crash mid-run simply resumes.

PROOF RECIPE (scratch copy of the live database; never the live one)
    createdb -T template0 funnelswift_closure_proof && pg_dump ... | psql funnelswift_closure_proof
    closure_erasure.py --db-url postgres://.../funnelswift_closure_proof \
                       --expected-db funnelswift_closure_proof --days 30            # dry run
    closure_erasure.py --db-url ... --expected-db funnelswift_closure_proof --apply
    docs/closure-erasure.md carries the whole recipe, the reversal path and the audit query.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from typing import NoReturn

ENV_FILE_DEFAULT = "/etc/swift/env/funnelswift.env"
EXPECTED_DB_DEFAULT = "funnelswift"
WINDOW_FLOOR_DAYS = 30
FIELD_SEP = "\x1f"


def log(message: str) -> None:
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    print(f"{ts} {message}", flush=True)


def die(message: str, code: int = 3) -> NoReturn:
    log(f"FATAL {message}")
    sys.exit(code)


def last_line(text: str) -> str:
    """The one line of a psql error worth logging (psql puts the cause last)."""
    lines = [line.strip() for line in (text or "").splitlines() if line.strip()]
    return lines[-1] if lines else "no diagnostic from psql"


def resolve_db_url(explicit: str | None, env_file: str) -> str:
    if explicit:
        return explicit.strip()
    from_env = os.environ.get("DATABASE_URL", "").strip()
    if from_env:
        return from_env
    if not os.path.isfile(env_file):
        die(f"no --db-url, no DATABASE_URL in the environment, and {env_file} does not exist")
    try:
        with open(env_file, "r", encoding="utf-8") as handle:
            for line in handle:
                line = line.strip()
                if line.startswith("DATABASE_URL="):
                    value = line.split("=", 1)[1].strip().strip('"').strip("'")
                    if value:
                        return value
    except OSError as exc:
        die(f"cannot read {env_file}: {exc}")
    die(f"no DATABASE_URL line in {env_file}")


def psql(db_url: str, sql: str, *, tuples_only: bool = True) -> str:
    """Run one statement. Returns stdout; raises CalledProcessError with stderr attached."""
    binary = shutil.which("psql")
    if not binary:
        die("psql is not installed on this host")
    argv = [binary, db_url, "-X", "-q", "-v", "ON_ERROR_STOP=1"]
    if tuples_only:
        argv += ["-A", "-t", "-F", FIELD_SEP]
    argv += ["-c", sql]
    return subprocess.run(
        argv, capture_output=True, text=True, timeout=180, check=True
    ).stdout


def psql_json(db_url: str, sql: str) -> object:
    return json.loads(psql(db_url, sql).strip() or "null")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Erase workspaces whose cancellation window has run out (FunnelSwift standard §2)."
    )
    parser.add_argument("--apply", action="store_true",
                        help="actually erase (default is a dry run that changes nothing)")
    parser.add_argument("--days", type=int, default=WINDOW_FLOOR_DAYS,
                        help="closure window in days; the SQL floors it at 30")
    parser.add_argument("--limit", type=int, default=100,
                        help="maximum workspaces handled in one run (1..500)")
    parser.add_argument("--env-file", default=ENV_FILE_DEFAULT,
                        help=f"file to read DATABASE_URL from when none is given ({ENV_FILE_DEFAULT})")
    parser.add_argument("--db-url", default=None,
                        help="database URL; for a scratch-copy proof only — never printed")
    parser.add_argument("--expected-db", default=EXPECTED_DB_DEFAULT,
                        help=f"refuse to run against any other database ({EXPECTED_DB_DEFAULT})")
    args = parser.parse_args(argv)

    days = max(args.days, WINDOW_FLOOR_DAYS)
    if args.days < WINDOW_FLOOR_DAYS:
        log(f"NOTE --days {args.days} ignored: the standard's window is {WINDOW_FLOOR_DAYS} days and "
            f"the census floors it; running with {days}")
    if not 1 <= args.limit <= 500:
        die(f"--limit must be between 1 and 500 (got {args.limit})")

    mode = "APPLY" if args.apply else "DRY-RUN"
    db_url = resolve_db_url(args.db_url, args.env_file)

    try:
        database = psql(db_url, "SELECT current_database()").strip()
    except subprocess.CalledProcessError as exc:
        die(f"cannot reach the database: {last_line(exc.stderr)}")
    if database != args.expected_db:
        die(f"refusing to run against database '{database}' — expected '{args.expected_db}' "
            f"(a closure erasure must never touch a database the operator did not name)")

    # 1. CENSUS — one statement, all policy in SQL.
    census_sql = (
        "SELECT tenant_id, workspace, closed_users, last_closed_at "
        f"FROM funnelswift_closures_due({days}) "
        f"ORDER BY last_closed_at LIMIT {args.limit}"
    )
    try:
        raw = psql(db_url, census_sql)
    except subprocess.CalledProcessError as exc:
        die(f"the census failed: {last_line(exc.stderr)}")

    due = []
    for line in raw.splitlines():
        if not line:
            continue
        parts = line.split(FIELD_SEP)
        if len(parts) != 4:
            die(f"unexpected census row shape ({len(parts)} fields) — refusing to guess")
        tenant_id, workspace, closed_users, last_closed = parts
        due.append({
            "tenant_id": tenant_id,
            "workspace": workspace,
            "closed_users": int(closed_users),
            "last_closed_at": last_closed,
        })

    log(f"mode={mode} database={database} window={days}d limit={args.limit} due={len(due)}")
    for row in due:
        log(f"DUE workspace={row['workspace']!r} tenant={row['tenant_id']} "
            f"closed_users={row['closed_users']} last_closed_at={row['last_closed_at']}")

    if not args.apply:
        log(f"DRY-RUN complete: {len(due)} workspace(s) would be erased; nothing was changed. "
            f"Re-run with --apply to erase them.")
        print("SUMMARY " + json.dumps({
            "mode": "dry-run", "database": database, "window_days": days,
            "due": len(due), "erased": [], "failed": [],
        }))
        return 0

    # 2. ERASE — one statement per workspace, so each commits or fails on its own.
    erased: list[dict] = []
    failed: list[dict] = []
    for row in due:
        tenant_id = row["tenant_id"]
        # tenant_id came from the census as a uuid; quote it as a uuid literal (no user text).
        if not tenant_id or any(ch not in "0123456789abcdef-" for ch in tenant_id.lower()):
            failed.append({"tenant_id": tenant_id, "workspace": row["workspace"],
                           "error": "census returned a non-uuid tenant id"})
            log(f"FAILED workspace={row['workspace']!r} :: census returned a non-uuid tenant id")
            continue
        sql = (f"SELECT funnelswift_erase_tenant('{tenant_id}'::uuid, NULL, "
               f"'closure-{days}d')")
        try:
            outcome = psql_json(db_url, sql)
        except subprocess.CalledProcessError as exc:
            detail = last_line(exc.stderr)
            failed.append({"tenant_id": tenant_id, "workspace": row["workspace"], "error": detail})
            log(f"FAILED workspace={row['workspace']!r} tenant={tenant_id} :: {detail}")
            continue
        except json.JSONDecodeError as exc:
            failed.append({"tenant_id": tenant_id, "workspace": row["workspace"],
                           "error": f"unreadable purge result: {exc}"})
            log(f"FAILED workspace={row['workspace']!r} tenant={tenant_id} :: unreadable purge result")
            continue
        if not isinstance(outcome, dict) or outcome.get("status") != "erased":
            failed.append({"tenant_id": tenant_id, "workspace": row["workspace"],
                           "error": "purge did not report status=erased"})
            log(f"FAILED workspace={row['workspace']!r} tenant={tenant_id} :: purge did not report erased")
            continue
        erased.append({
            "tenant_id": tenant_id,
            "workspace": row["workspace"],
            "rows_deleted": outcome.get("rows_deleted"),
            "invoices_retained": outcome.get("invoices_retained"),
            "reason": outcome.get("reason"),
        })
        log(f"ERASED workspace={row['workspace']!r} tenant={tenant_id} "
            f"rows={outcome.get('rows_deleted')} invoices_kept={outcome.get('invoices_retained')} "
            f"reason={outcome.get('reason')}")

    truncated = len(due) == args.limit
    if truncated:
        log(f"NOTE the census was capped at --limit {args.limit}; more workspaces may be due, "
            f"and the next run will take them")
    log(f"APPLY complete: erased={len(erased)} failed={len(failed)}")
    print("SUMMARY " + json.dumps({
        "mode": "apply", "database": database, "window_days": days, "due": len(due),
        "erased": erased, "failed": failed, "truncated": truncated,
    }))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
