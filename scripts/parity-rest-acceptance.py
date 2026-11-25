#!/usr/bin/env python3
"""REST+upload parity proof for FunnelSwift (fleet standard §10, kanban t_3a7f70c1).

The MCP door (`scripts/mcp-parity-acceptance.py`) proves the 30 write tools. This script proves the
console's OTHER action families, which the product serves as REST routes: the same account key
performs the action, and the row is read back from the database to show it changed.

  settings + lead stages, suppressions, product categories, workspaces, own profile name,
  own avatar upload, workspace logo upload, provider-key test, email-config test,
  checkout create, key mint/reveal/rotate, own password change, account close/erase

plus the boundary: a platform-operator surface answers 403 to a customer key, and another
workspace's key id is never rotatable (404).

Usage: python3 scripts/parity-rest-acceptance.py [--base http://127.0.0.1:8080]
"""

import argparse
import base64
import hashlib
import json
import os
import secrets
import subprocess
import sys
import urllib.error
import urllib.request
import uuid

ENV_FILE = "/etc/swift/env/funnelswift.env"
TAG = "parity-rest-acceptance"
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)


def database_url() -> str:
    with open(ENV_FILE) as handle:
        for line in handle:
            if line.startswith("DATABASE_URL="):
                return line.strip().split("=", 1)[1]
    raise SystemExit("DATABASE_URL not found in " + ENV_FILE)


def sql(query: str) -> str:
    out = subprocess.run(
        ["psql", database_url(), "-At", "-v", "ON_ERROR_STOP=1", "-c", query],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        raise SystemExit("SQL failed: " + out.stderr.strip() + "\n" + query)
    return out.stdout.strip()


def q(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def http(base, method, path, key, body=None, raw=None, content_type="application/json"):
    data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
    request = urllib.request.Request(
        base.rstrip("/") + path,
        data=data,
        method=method,
        headers={"Authorization": "Bearer " + key, "Content-Type": content_type},
    )
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            body = response.read().decode()
            return response.status, body
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def account(stamp, suffix, base):
    tenant = str(uuid.uuid4())
    user = str(uuid.uuid4())
    key_id = str(uuid.uuid4())
    secret = "fs_" + secrets.token_hex(32)
    sql(
        "INSERT INTO tenants (id, name, slug, email, status, probe_harness) VALUES "
        f"({q(tenant)}, {q('Parity REST ' + suffix + ' ' + stamp)}, "
        f"{q('parity-rest-' + suffix + '-' + stamp)}, {q('probe@example.invalid')}, 'active', {q(TAG)});"
    )
    sql(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active) VALUES "
        f"({q(user)}, {q(tenant)}, {q('probe-' + suffix + '-' + stamp + '@example.invalid')}, "
        f"'not-a-login', 'Parity REST {suffix}', 'user', true);"
    )
    sql(
        "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status) VALUES "
        f"(gen_random_uuid(), {q(tenant)}, (SELECT id FROM plans WHERE slug = 'agency'), 'active');"
    )
    sql(
        "INSERT INTO api_keys (id, tenant_id, name, key_prefix, key_hash, key_sealed, created_by) "
        f"VALUES ({q(key_id)}, {q(tenant)}, 'parity rest key', {q(secret[:8])}, "
        f"{q(hashlib.sha256(secret.encode()).hexdigest())}, 'enc:v1:probe-not-revealable', {q(user)});"
    )
    return tenant, user, key_id, secret


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8080")
    args = parser.parse_args()
    base = args.base
    stamp = secrets.token_hex(4)
    checks = []

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})
        print(("PASS  " if ok else "FAIL  ") + name + (("  — " + detail) if detail else ""))

    tenant_a, user_a, key_a, secret_a = account(stamp, "a", base)
    tenant_b, user_b, key_b, _ = account(stamp, "b", base)

    try:
        # ── every console READ answers the key ──────────────────────────────────
        for label, path in [
            ("settings", "/api/v1/settings"),
            ("suppressions", "/api/v1/settings/suppressions"),
            ("product categories", "/api/v1/product-categories"),
            ("workspaces", "/api/v1/workspaces"),
            ("private-email domains", "/api/v1/private-email/domains"),
            ("private-email mailboxes", "/api/v1/private-email/boxes"),
            ("team", "/api/v1/team/members"),
            ("API keys (list only)", "/api/v1/api-keys"),
            ("support requests", "/api/v1/support-requests"),
            ("CoreSwift integration status", "/api/v1/integrations/coreswift/status"),
            ("my affiliate", "/api/v1/affiliate-links"),
            ("card scanner history", "/api/v1/ocr/scans"),
            ("plans (price list)", "/api/v1/plans"),
            ("Kinetic site meta", "/api/v1/kinetic/site-meta"),
            ("dashboard", "/api/v1/dashboard/stats"),
            ("mail log", "/api/v1/settings/mail-log"),
            ("tag change log", "/api/v1/tag-change-log"),
            ("webhook events", "/api/v1/webhook-events"),
        ]:
            status, body = http(base, "GET", path, secret_a)
            check(f"key reads {label}", status == 200, f"{status} {body[:80]}")

        # ── writes, each read back from the database ────────────────────────────
        status, body = http(base, "PUT", "/api/v1/settings", secret_a,
                            {"key": "lead_stages", "value": ["New", "Contacted", "Won"]})
        stored = sql(f"SELECT value::text FROM tenant_settings WHERE tenant_id = {q(tenant_a)} "
                     "AND key = 'lead_stages'")
        check("lead stages saved by key (Lead Stages screen)", status == 200 and "Won" in stored,
              f"{status} {stored[:60]}")

        status, body = http(base, "POST", "/api/v1/product-categories", secret_a,
                            {"name": "Parity cat " + stamp})
        cat = sql(f"SELECT id FROM product_categories WHERE tenant_id = {q(tenant_a)} "
                  f"AND name = {q('Parity cat ' + stamp)}")
        check("product category created by key", status == 201 and bool(cat), f"{status} {cat[:40]}")
        if cat:
            status, _ = http(base, "PUT", f"/api/v1/product-categories/{cat}", secret_a,
                             {"name": "Parity cat 2 " + stamp})
            renamed = sql(f"SELECT name FROM product_categories WHERE id = {q(cat)}")
            check("product category renamed by key",
                  status == 200 and renamed == "Parity cat 2 " + stamp, f"{status} {renamed[:40]}")

        status, _ = http(base, "POST", "/api/v1/settings/suppressions", secret_a,
                         {"address": f"parity-{stamp}@example.invalid"})
        suppressed = sql("SELECT count(*) FROM email_suppressions WHERE tenant_id = "
                         f"{q(tenant_a)} AND lower(address) = {q('parity-' + stamp + '@example.invalid')}")
        check("suppression added by key", status == 200 and suppressed == "1", f"{status} rows={suppressed}")

        status, _ = http(base, "POST", "/api/v1/workspaces", secret_a, {"name": "Parity ws " + stamp})
        ws = sql(f"SELECT count(*) FROM workspaces WHERE tenant_id = {q(tenant_a)} "
                 f"AND name = {q('Parity ws ' + stamp)}")
        check("workspace created by key", status in (200, 201) and ws == "1", f"{status} rows={ws}")

        status, _ = http(base, "PUT", "/api/v1/auth/profile", secret_a, {"name": "Parity name " + stamp})
        named = sql(f"SELECT name FROM users WHERE id = {q(user_a)}")
        check("own profile name changed by key", status == 200 and named == "Parity name " + stamp,
              f"{status} {named[:40]}")

        status, _ = http(base, "POST", "/api/v1/auth/avatar", secret_a, raw=PNG,
                         content_type="image/png")
        avatar = sql(f"SELECT octet_length(bytes) FROM user_avatars WHERE user_id = {q(user_a)}")
        check("own avatar uploaded by key", status == 200 and avatar != "", f"{status} bytes={avatar}")

        status, _ = http(base, "POST", "/api/v1/settings/branding/logo", secret_a, raw=PNG,
                         content_type="image/png")
        logo = sql(f"SELECT octet_length(bytes) FROM tenant_logos WHERE tenant_id = {q(tenant_a)}")
        check("workspace logo uploaded by key", status == 200 and logo != "", f"{status} bytes={logo}")

        status, body = http(base, "POST", "/api/v1/provider-keys/openai/test", secret_a, {})
        check("provider-key TEST button answers the key (no key stored → valid=false)",
              status == 200 and '"valid":false' in body, f"{status} {body[:80]}")

        status, body = http(base, "POST", "/api/v1/settings/email-config/test", secret_a, {})
        check("email-config TEST button answers the key", status == 200, f"{status} {body[:80]}")

        status, body = http(base, "POST", "/api/v1/checkout/create", secret_a, {"plan_slug": "agency"})
        check("checkout answers the key (refuses only because no payment provider is configured)",
              status == 424 and "payment_provider_not_configured" in body, f"{status}")

        status, body = http(base, "POST", "/api/v1/api-keys", secret_a, {"name": "second"})
        check("a key cannot create another key (session only, 403)", status == 403, f"{status}")

        status, body = http(base, "POST", f"/api/v1/api-keys/{key_a}/reveal", secret_a, {})
        check("key REVEAL needs the signed-in person (refused)", status in (401, 403), f"{status}")

        # ── the boundaries that stay shut ───────────────────────────────────────
        status, _ = http(base, "POST", "/api/v1/auth/close-account", secret_a,
                         {"password": "x", "confirm": "close"})
        check("account close refused to a key", status in (400, 401, 403), f"{status}")
        status, _ = http(base, "POST", "/api/v1/auth/erase-data", secret_a,
                         {"password": "x", "confirm": "erase"})
        check("data erasure refused to a key", status in (400, 401, 403), f"{status}")
        status, _ = http(base, "GET", "/api/v1/admin/plans", secret_a)
        check("platform plans refused to a customer key", status in (401, 403), f"{status}")
        status, _ = http(base, "GET", "/api/v1/integration/schema", secret_a)
        check("operator integration schema refused to a customer key", status in (401, 403), f"{status}")
        status, _ = http(base, "PUT", "/api/v1/auth/password", secret_a,
                         {"current_password": "wrong-password", "new_password": "another-long-one1"})
        check("own password change needs the current password, which a key does not have",
              status in (400, 401), f"{status}")

        # rotate is a session act: it hands back a new secret, so a key must never do it
        status, body = http(base, "POST", f"/api/v1/api-keys/{key_a}/regenerate", secret_a, {})
        check("own key cannot be rotated with a key (403, session only)", status == 403, f"{status}")

        # a foreign key id is refused at the same door (the handler scopes by tenant too: measured
        # 404 for another workspace's id before this guard was added)
        status, body = http(base, "POST", f"/api/v1/api-keys/{key_b}/regenerate", secret_a, {})
        check("another workspace's key id is refused", status in (403, 404), f"{status}")
    finally:
        sql(f"DELETE FROM tenants WHERE id IN ({q(tenant_a)}, {q(tenant_b)});")

    residue = sql(f"SELECT count(*) FROM tenants WHERE probe_harness = {q(TAG)}")
    check("teardown leaves no probe account", residue == "0", residue)

    passed = sum(1 for c in checks if c["ok"])
    failed = len(checks) - passed
    print(f"\n=== RESULT: {passed} passed, {failed} failed ===")
    report = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..",
                          "parity-rest-acceptance.json")
    with open(report, "w") as handle:
        json.dump({"checks": checks, "passed": passed, "failed": failed}, handle, indent=2)
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
