#!/usr/bin/env python3
"""Acceptance proof for FunnelSwift's §9 write tier (kanban t_03af3510).

Proves, against the LIVE door and the LIVE REST routes, with ONE account's own key:

  * tools/list offers 23 tools (11 reads + 12 writes) and no operator tool;
  * per entity — funnel, page, lead, tag — create -> read back -> update -> archive;
  * a FOREIGN id (a real row in another account) answers not_found, never 403;
  * a platform-admin route refuses the same key with 403;
  * every write left an activity_log row naming the key id and the row id;
  * the REST twins (same key auth, same scoping) create, update and refuse a foreign id too;
  * teardown leaves no residue.

Two probe accounts are created with `tenants.probe_harness` set (the fleet's marker-at-creation
rule). The probe key's `key_hash` is sha256(secret) — the app's own stored form — so the proof needs
no reveal and never touches a real account's key. Both accounts are deleted at the end; every row
this script writes lives under them and cascades.

Usage: python3 scripts/mcp-s9-write-acceptance.py [--base http://127.0.0.1:8080]
"""

import argparse
import hashlib
import json
import os
import re
import secrets
import subprocess
import sys
import urllib.error
import urllib.request
import uuid

ENV_FILE = "/etc/swift/env/funnelswift.env"
TAG = "mcp-s9-write-proof"


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


class Mcp:
    def __init__(self, base: str, key: str):
        self.url = base.rstrip("/") + "/mcp"
        self.key = key
        self.next_id = 1

    def call(self, method: str, params=None):
        body = json.dumps(
            {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params or {}}
        ).encode()
        request = urllib.request.Request(
            self.url,
            data=body,
            headers={
                "Content-Type": "application/json",
                "Authorization": "Bearer " + self.key,
            },
        )
        self.next_id += 1
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)

    def tool(self, name: str, arguments=None):
        """Returns (ok, payload_or_error)."""
        response = self.call("tools/call", {"name": name, "arguments": arguments or {}})
        if "error" in response:
            return False, response["error"]
        result = response.get("result", {})
        if result.get("isError"):
            text = result.get("content", [{}])[0].get("text", "")
            return False, text
        content = result.get("content") or [{}]
        text = content[0].get("text", "")
        try:
            return True, json.loads(text)
        except json.JSONDecodeError:
            return True, text


def http(base: str, method: str, path: str, key: str, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Authorization": "Bearer " + key, "Content-Type": "application/json"}
    request = urllib.request.Request(base.rstrip("/") + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read().decode()
            return response.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as error:
        raw = error.read().decode()
        try:
            parsed = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            parsed = raw
        return error.code, parsed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8080")
    args = parser.parse_args()

    stamp = secrets.token_hex(4)
    checks = []

    def check(name: str, ok: bool, detail: str = ""):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})
        print(("PASS  " if ok else "FAIL  ") + name + (("  — " + detail) if detail else ""))

    tenant_a = str(uuid.uuid4())
    tenant_b = str(uuid.uuid4())
    user_a = str(uuid.uuid4())
    key_a = str(uuid.uuid4())
    # The REST boundary's key shape: `fs_` + 64 hex characters (handlers::api_keys::is_api_key).
    secret = "fs_" + secrets.token_hex(32)
    prefix = secret[:8]
    key_hash = hashlib.sha256(secret.encode()).hexdigest()

    sql(
        "INSERT INTO tenants (id, name, slug, email, status, probe_harness) VALUES "
        f"({q(tenant_a)}, {q('MCP s9 proof A ' + stamp)}, {q('mcp-s9-a-' + stamp)}, "
        f"{q('probe@example.invalid')}, 'active', {q(TAG)}), "
        f"({q(tenant_b)}, {q('MCP s9 proof B ' + stamp)}, {q('mcp-s9-b-' + stamp)}, "
        f"{q('probe@example.invalid')}, 'active', {q(TAG)});"
    )
    sql(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active) VALUES "
        f"({q(user_a)}, {q(tenant_a)}, {q('probe-a-' + stamp + '@example.invalid')}, 'not-a-login', "
        f"'MCP proof A', 'user', true);"
    )
    # Give the probe account the product's free tier: the console gives every signup a plan, and a
    # plan-less account is legitimately refused a card ("Cards is not included in ..."), which is a
    # different answer than the one this proof is about.
    sql(
        "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status) VALUES "
        f"(gen_random_uuid(), {q(tenant_a)}, "
        "(SELECT id FROM plans WHERE slug = 'kinetic-free'), 'active');"
    )
    sql(
        "INSERT INTO api_keys (id, tenant_id, name, key_prefix, key_hash, key_sealed, created_by) "
        f"VALUES ({q(key_a)}, {q(tenant_a)}, 'MCP s9 proof key', {q(prefix)}, {q(key_hash)}, "
        f"'enc:v1:probe-not-revealable', {q(user_a)});"
    )
    # A REAL row in the other account: the foreign-id answer must be not_found, not forbidden.
    foreign_funnel = str(uuid.uuid4())
    sql(
        "INSERT INTO funnels (id, tenant_id, name, slug) VALUES "
        f"({q(foreign_funnel)}, {q(tenant_b)}, 'Foreign account funnel', {q('foreign-' + stamp)});"
    )

    try:
        mcp = Mcp(args.base, secret)
        mcp.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "s9-proof", "version": "1"}})
        listed = mcp.call("tools/list").get("result", {}).get("tools", [])
        names = sorted(t["name"] for t in listed)
        check("tools/list offers 23 tools", len(names) == 23, str(len(names)))
        check(
            "the write tier is present",
            all(n in names for n in ["create_funnel", "update_funnel", "archive_funnel",
                                     "create_page", "update_page", "archive_page",
                                     "create_lead", "update_lead", "archive_lead",
                                     "create_tag", "update_tag", "archive_tag"]),
            ",".join(names),
        )
        check("no operator/admin tool is advertised",
              not any(n.startswith("admin_") or "workspace" in n for n in names))

        # ── funnel ────────────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_funnel", {"name": "S9 proof funnel " + stamp,
                                                "headline": "Created by the proof"})
        funnel_id = created.get("id") if isinstance(created, dict) else None
        check("funnel create", ok and bool(funnel_id), str(created)[:160])
        ok, listing = mcp.tool("list_funnels", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == funnel_id), None)
        check("funnel read back", ok and row is not None and row.get("headline_excerpt") == "Created by the proof",
              json.dumps(row)[:160] if row else "not found")
        ok, updated = mcp.tool("update_funnel", {"id": funnel_id, "headline": "Updated by the proof"})
        ok2, listing = mcp.tool("list_funnels", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == funnel_id), None)
        check("funnel update", ok and row is not None and row.get("headline_excerpt") == "Updated by the proof")
        ok, _ = mcp.tool("archive_funnel", {"id": funnel_id})
        ok2, listing = mcp.tool("list_funnels", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == funnel_id), None)
        check("funnel archive (row kept, is_active false)",
              ok and row is not None and row.get("active") is False, json.dumps(row)[:160] if row else "")

        # ── page (kinetic card) ──────────────────────────────────────────────────
        ok, created = mcp.tool("create_page", {"title": "S9 proof page " + stamp})
        page_id = created.get("id") if isinstance(created, dict) else None
        check("page create", ok and bool(page_id), str(created)[:160])
        ok, listing = mcp.tool("list_pages", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == page_id), None)
        check("page read back", ok and row is not None, json.dumps(row)[:160] if row else "not found")
        ok, updated = mcp.tool("update_page", {"id": page_id, "bio": "Updated by the proof"})
        ok2, detail = mcp.tool("get_page", {"id": page_id})
        check("page update", ok and ok2 and "Updated by the proof" in json.dumps(detail))
        ok, _ = mcp.tool("archive_page", {"id": page_id})
        sql_state = sql(f"SELECT is_active FROM kinetic_cards WHERE id = {q(page_id)}") if page_id else "?"
        check("page archive (is_active false)", ok and sql_state == "f", sql_state)

        # ── lead ─────────────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_lead", {"name": "S9 proof lead", "email": "lead@example.invalid",
                                              "source": "mcp-proof"})
        lead_id = created.get("id") if isinstance(created, dict) else None
        check("lead create", ok and bool(lead_id), str(created)[:160])
        ok, listing = mcp.tool("list_leads", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == lead_id), None)
        check("lead read back", ok and row is not None and row.get("name") == "S9 proof lead")
        ok, _ = mcp.tool("update_lead", {"id": lead_id, "phone": "+13055550123"})
        ok2, listing = mcp.tool("list_leads", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == lead_id), None)
        check("lead update", ok and row is not None and row.get("phone") == "+13055550123")
        ok, _ = mcp.tool("archive_lead", {"id": lead_id})
        ok2, listing = mcp.tool("list_leads", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == lead_id), None)
        check("lead archive (status archived, row kept)", ok and row is not None and row.get("status") == "archived")

        # ── tag ──────────────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_tag", {"name": "S9 proof tag " + stamp, "color": "#3B82F6"})
        tag_id = created.get("id") if isinstance(created, dict) else None
        check("tag create", ok and bool(tag_id), str(created)[:160])
        ok, listing = mcp.tool("list_tags", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == tag_id), None)
        check("tag read back", ok and row is not None and row.get("name") == "S9 proof tag " + stamp)
        ok, _ = mcp.tool("update_tag", {"id": tag_id, "color": "#22C55E"})
        ok2, listing = mcp.tool("list_tags", {})
        row = next((i for i in listing.get("items", []) if i.get("id") == tag_id), None)
        check("tag update", ok and row is not None and row.get("color") == "#22C55E")
        ok, refusal = mcp.tool("archive_tag", {"id": tag_id})
        check("tag archive without hard_delete is refused", not ok, str(refusal)[:120])
        ok, _ = mcp.tool("archive_tag", {"id": tag_id, "hard_delete": True})
        ok2, listing = mcp.tool("list_tags", {})
        gone = all(i.get("id") != tag_id for i in listing.get("items", []))
        check("tag archive with hard_delete=true deletes the row", ok and gone)

        # ── foreign id + admin refusal ──────────────────────────────────────────
        ok, refusal = mcp.tool("update_funnel", {"id": foreign_funnel, "headline": "should not land"})
        check("foreign funnel id answers not_found", not ok and "not_found" in str(refusal), str(refusal)[:160])
        ok, refusal = mcp.tool("archive_funnel", {"id": foreign_funnel})
        check("foreign archive answers not_found", not ok and "not_found" in str(refusal), str(refusal)[:160])

        status, body = http(args.base, "GET", "/api/v1/admin/invoices", secret)
        check("platform-admin route refuses the key (403)", status == 403, f"{status} {str(body)[:120]}")

        # ── audits ──────────────────────────────────────────────────────────────
        actions = sql(
            "SELECT action FROM activity_log WHERE tenant_id = "
            f"{q(tenant_a)} AND user_id = {q('mcp-key:' + key_a)} "
            "AND metadata->>'outcome' = 'ok' ORDER BY created_at"
        ).splitlines()
        expected_actions = [
            "funnel.created", "funnel.updated", "funnel.archived",
            "page.created", "page.updated", "page.archived",
            "lead.created", "lead.updated", "lead.archived",
            "tag.created", "tag.updated", "tag.deleted",
        ]
        check("every write audited with the key id", actions == expected_actions,
              ",".join(actions))

        linked = sql(
            "SELECT count(*) FROM activity_log WHERE tenant_id = " + q(tenant_a)
            + " AND user_id = " + q("mcp-key:" + key_a)
            + " AND entity_id IS NOT NULL AND metadata->>'outcome' = 'ok'"
        )
        check("audit rows carry the entity id", linked == "12", linked)
        refused = sql(
            "SELECT count(*) FROM activity_log WHERE tenant_id = " + q(tenant_a)
            + " AND user_id = " + q("mcp-key:" + key_a)
            + " AND metadata->>'outcome' = 'not_found'"
        )
        check("refused writes are audited too", int(refused) >= 2, refused)

        # ── REST twins: same key, same scoping ──────────────────────────────────
        status, body = http(args.base, "POST", "/api/v1/funnels", secret,
                            {"name": "REST proof funnel " + stamp})
        rest_id = (body or {}).get("id") if isinstance(body, dict) else None
        check("REST create with the key (201)", status == 201 and bool(rest_id), f"{status}")
        status, body = http(args.base, "PUT", f"/api/v1/funnels/{rest_id}", secret,
                            {"headline": "REST updated"})
        check("REST update with the key (200)", status == 200, f"{status} {str(body)[:120]}")
        status, body = http(args.base, "GET", f"/api/v1/funnels/{rest_id}", secret)
        check("REST read back", status == 200 and (body or {}).get("headline") == "REST updated")
        status, body = http(args.base, "PUT", f"/api/v1/funnels/{foreign_funnel}", secret,
                            {"headline": "no"})
        check("REST foreign id answers 404", status == 404, f"{status}")
    finally:
        sql(f"DELETE FROM tenants WHERE id IN ({q(tenant_a)}, {q(tenant_b)});")

    residue = sql(
        f"SELECT count(*) FROM tenants WHERE probe_harness = {q(TAG)}"
    )
    check("teardown leaves no probe account", residue == "0", residue)

    passed = sum(1 for c in checks if c["ok"])
    failed = len(checks) - passed
    print(f"\n=== RESULT: {passed} passed, {failed} failed ===")
    report = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..",
                          "mcp-s9-write-acceptance.json")
    with open(report, "w") as handle:
        json.dump({"checks": checks, "passed": passed, "failed": failed}, handle, indent=2)
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
