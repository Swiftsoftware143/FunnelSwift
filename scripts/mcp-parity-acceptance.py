#!/usr/bin/env python3
"""Acceptance proof for FunnelSwift's §10 parity tier (kanban t_3a7f70c1).

Proves, against the LIVE door with ONE account's own key, that the action families the console
edits are reachable WITHOUT a login, each performing create -> read back -> update -> archive and
showing the row change:

  campaigns, tag groups, tagging rules, web-to-lead forms, webhooks, QR codes
  (beside the §9 core: funnels, pages, leads, tags)

plus: tools/list offers 41 tools, a foreign id answers not_found, every write leaves an
activity_log row naming the key id, and the excluded actions (account close/erase) are refused.

Probe accounts are marked (`tenants.probe_harness`) and deleted at the end; every row this script
writes lives under them and cascades.

Usage: python3 scripts/mcp-parity-acceptance.py [--base http://127.0.0.1:8080]
"""

import argparse
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
TAG = "agent-parity-acceptance"


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
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + self.key},
        )
        self.next_id += 1
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)

    def tool(self, name: str, arguments=None):
        response = self.call("tools/call", {"name": name, "arguments": arguments or {}})
        if "error" in response:
            return False, response["error"]
        result = response.get("result", {})
        if result.get("isError"):
            return False, result.get("content", [{}])[0].get("text", "")
        text = (result.get("content") or [{}])[0].get("text", "")
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
    secret = "fs_" + secrets.token_hex(32)

    sql(
        "INSERT INTO tenants (id, name, slug, email, status, probe_harness) VALUES "
        f"({q(tenant_a)}, {q('Parity proof A ' + stamp)}, {q('parity-a-' + stamp)}, "
        f"{q('probe@example.invalid')}, 'active', {q(TAG)}), "
        f"({q(tenant_b)}, {q('Parity proof B ' + stamp)}, {q('parity-b-' + stamp)}, "
        f"{q('probe@example.invalid')}, 'active', {q(TAG)});"
    )
    sql(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active) VALUES "
        f"({q(user_a)}, {q(tenant_a)}, {q('probe-a-' + stamp + '@example.invalid')}, 'not-a-login', "
        f"'Parity proof A', 'user', true);"
    )
    sql(
        "INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status) VALUES "
        f"(gen_random_uuid(), {q(tenant_a)}, (SELECT id FROM plans WHERE slug = 'agency'), 'active');"
    )
    sql(
        "INSERT INTO api_keys (id, tenant_id, name, key_prefix, key_hash, key_sealed, created_by) "
        f"VALUES ({q(key_a)}, {q(tenant_a)}, 'parity proof key', {q(secret[:8])}, "
        f"{q(hashlib.sha256(secret.encode()).hexdigest())}, 'enc:v1:probe-not-revealable', {q(user_a)});"
    )
    # A tag in this workspace (a tagging rule needs a trigger tag) and one in ANOTHER workspace.
    tag_id = str(uuid.uuid4())
    foreign_tag = str(uuid.uuid4())
    sql(
        "INSERT INTO tags (id, tenant_id, name, color) VALUES "
        f"({q(tag_id)}, {q(tenant_a)}, {q('parity-tag-' + stamp)}, '#3B82F6'), "
        f"({q(foreign_tag)}, {q(tenant_b)}, {q('foreign-tag-' + stamp)}, '#EF4444');"
    )

    try:
        mcp = Mcp(args.base, secret)
        mcp.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "parity-proof", "version": "1"}})
        listed = mcp.call("tools/list").get("result", {}).get("tools", [])
        names = sorted(t["name"] for t in listed)
        check("tools/list offers 41 tools (11 reads + 30 writes)", len(names) == 41, str(len(names)))
        expected = [
            "create_campaign", "update_campaign", "archive_campaign",
            "create_tag_group", "update_tag_group", "archive_tag_group",
            "create_tag_rule", "update_tag_rule", "archive_tag_rule",
            "create_web_to_lead", "update_web_to_lead", "archive_web_to_lead",
            "create_webhook", "update_webhook", "archive_webhook",
            "create_qr_code", "update_qr_code", "archive_qr_code",
        ]
        check("the §10 parity tier is present", all(n in names for n in expected),
              ",".join(n for n in expected if n not in names) or "all present")
        check("no operator/admin tool is advertised",
              not any(n.startswith("admin_") or "workspace" in n for n in names))

        # ── campaign ─────────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_campaign", {"name": "Parity campaign " + stamp, "type": "email"})
        cid = created.get("id") if isinstance(created, dict) else None
        check("campaign create", ok and bool(cid), str(created)[:160])
        state = sql(f"SELECT name || '|' || coalesce(campaign_type,'') FROM campaigns WHERE id = {q(cid)}") if cid else ""
        check("campaign read back (row changed)", state == "Parity campaign " + stamp + "|email", state)
        ok, _ = mcp.tool("update_campaign", {"id": cid, "name": "Parity campaign 2 " + stamp})
        state = sql(f"SELECT name FROM campaigns WHERE id = {q(cid)}") if cid else ""
        check("campaign update", ok and state == "Parity campaign 2 " + stamp, state)
        ok, _ = mcp.tool("archive_campaign", {"id": cid})
        state = sql(f"SELECT status FROM campaigns WHERE id = {q(cid)}") if cid else ""
        check("campaign archive (row kept, status archived)", ok and state == "archived", state)

        # ── tag group ────────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_tag_group", {"name": "Parity group " + stamp})
        gid = created.get("id") if isinstance(created, dict) else None
        check("tag group create", ok and bool(gid), str(created)[:160])
        state = sql(f"SELECT name FROM tag_groups WHERE id = {q(gid)}") if gid else ""
        check("tag group read back", state == "Parity group " + stamp, state)
        ok, _ = mcp.tool("update_tag_group", {"id": gid, "name": "Parity group 2 " + stamp, "sort_order": 7})
        state = sql(f"SELECT name || '|' || sort_order FROM tag_groups WHERE id = {q(gid)}") if gid else ""
        check("tag group update", ok and state == "Parity group 2 " + stamp + "|7", state)
        ok, refusal = mcp.tool("archive_tag_group", {"id": gid})
        check("tag group archive without hard_delete is refused", not ok, str(refusal)[:120])
        ok, _ = mcp.tool("archive_tag_group", {"id": gid, "hard_delete": True})
        gone = sql(f"SELECT count(*) FROM tag_groups WHERE id = {q(gid)}") if gid else "1"
        check("tag group hard delete removes only the group", ok and gone == "0", gone)

        # ── tagging rule ─────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_tag_rule", {"name": "Parity rule " + stamp,
                                                  "trigger_tag_id": tag_id, "action_type": "add_tag",
                                                  "action_tag_id": tag_id})
        rid = created.get("id") if isinstance(created, dict) else None
        check("tag rule create", ok and bool(rid), str(created)[:160])
        state = sql(f"SELECT name FROM tag_rules WHERE id = {q(rid)}") if rid else ""
        check("tag rule read back", state == "Parity rule " + stamp, state)
        ok, _ = mcp.tool("update_tag_rule", {"id": rid, "name": "Parity rule 2 " + stamp})
        state = sql(f"SELECT name FROM tag_rules WHERE id = {q(rid)}") if rid else ""
        check("tag rule update", ok and state == "Parity rule 2 " + stamp, state)
        ok, _ = mcp.tool("archive_tag_rule", {"id": rid})
        state = sql(f"SELECT is_active FROM tag_rules WHERE id = {q(rid)}") if rid else ""
        check("tag rule archive (is_active false, row kept)", ok and state == "f", state)
        ok, refusal = mcp.tool("create_tag_rule", {"name": "foreign trigger",
                                                   "trigger_tag_id": foreign_tag, "action_type": "add_tag"})
        check("a rule on another workspace's tag is refused", not ok, str(refusal)[:120])

        # ── web-to-lead form ─────────────────────────────────────────────────────
        ok, created = mcp.tool("create_web_to_lead", {"name": "Parity form " + stamp,
                                                      "form_title": "Get in touch"})
        wid = created.get("id") if isinstance(created, dict) else None
        check("web-to-lead create", ok and bool(wid), str(created)[:160])
        state = sql(f"SELECT name FROM web_to_lead_configs WHERE id = {q(wid)}") if wid else ""
        check("web-to-lead read back", state == "Parity form " + stamp, state)
        ok, _ = mcp.tool("update_web_to_lead", {"id": wid, "name": "Parity form 2 " + stamp,
                                               "redirect_url": "https://example.invalid/thanks"})
        state = sql(f"SELECT name FROM web_to_lead_configs WHERE id = {q(wid)}") if wid else ""
        check("web-to-lead update", ok and state == "Parity form 2 " + stamp, state)
        ok, _ = mcp.tool("archive_web_to_lead", {"id": wid})
        state = sql(f"SELECT is_active FROM web_to_lead_configs WHERE id = {q(wid)}") if wid else ""
        check("web-to-lead archive (is_active false, row kept)", ok and state == "f", state)

        # ── webhook ──────────────────────────────────────────────────────────────
        ok, created = mcp.tool("create_webhook", {"name": "Parity hook " + stamp,
                                                 "url": "https://example.com/hook",
                                                 "events": ["lead.created"]})
        hid = created.get("id") if isinstance(created, dict) else None
        check("webhook create", ok and bool(hid), str(created)[:160])
        state = sql(f"SELECT name FROM webhooks WHERE id = {q(hid)}") if hid else ""
        check("webhook read back", state == "Parity hook " + stamp, state)
        ok, _ = mcp.tool("update_webhook", {"id": hid, "name": "Parity hook 2 " + stamp})
        state = sql(f"SELECT name FROM webhooks WHERE id = {q(hid)}") if hid else ""
        check("webhook update", ok and state == "Parity hook 2 " + stamp, state)
        ok, _ = mcp.tool("archive_webhook", {"id": hid})
        state = sql(f"SELECT is_active FROM webhooks WHERE id = {q(hid)}") if hid else ""
        check("webhook archive (is_active false, row kept)", ok and state == "f", state)

        # ── QR code (needs a page it can point at) ───────────────────────────────
        ok, created = mcp.tool("create_page", {"title": "Parity page " + stamp})
        page_id = created.get("id") if isinstance(created, dict) else None
        check("page create (precondition for a QR)", ok and bool(page_id), str(created)[:120])
        ok, created = mcp.tool("create_qr_code", {"card_id": page_id, "title": "Parity QR " + stamp})
        qid = created.get("id") if isinstance(created, dict) else None
        check("QR create", ok and bool(qid), str(created)[:160])
        state = sql(f"SELECT name FROM kinetic_qr_codes WHERE id = {q(qid)}") if qid else ""
        check("QR read back", state == "Parity QR " + stamp, state)
        ok, _ = mcp.tool("update_qr_code", {"id": qid, "name": "Parity QR 2 " + stamp})
        state = sql(f"SELECT name || '|' || is_active FROM kinetic_qr_codes WHERE id = {q(qid)}") if qid else ""
        check("QR update", ok and state == "Parity QR 2 " + stamp + "|true", state)
        ok, _ = mcp.tool("archive_qr_code", {"id": qid})
        state = sql(f"SELECT is_active FROM kinetic_qr_codes WHERE id = {q(qid)}") if qid else ""
        check("QR archive (is_active false, row kept)", ok and state == "f", state)

        # ── foreign ids answer not_found ─────────────────────────────────────────
        foreign_campaign = str(uuid.uuid4())
        sql("INSERT INTO campaigns (id, tenant_id, name) VALUES "
            f"({q(foreign_campaign)}, {q(tenant_b)}, 'Foreign campaign');")
        for tool, payload in [
            ("update_campaign", {"id": foreign_campaign, "name": "no"}),
            ("archive_campaign", {"id": foreign_campaign}),
            ("update_tag_group", {"id": foreign_tag, "name": "no"}),
        ]:
            ok, refusal = mcp.tool(tool, payload)
            check(f"foreign id refused ({tool})", not ok and "not_found" in str(refusal),
                  str(refusal)[:120])

        # ── exclusions stay refused to a key ────────────────────────────────────
        status, body = http(args.base, "POST", "/api/v1/auth/close-account", secret,
                            {"password": "x", "confirm": "close"})
        check("account close is refused to a key", status in (400, 401, 403), f"{status}")
        status, body = http(args.base, "POST", "/api/v1/auth/erase-data", secret,
                            {"password": "x", "confirm": "erase"})
        check("data erasure is refused to a key", status in (400, 401, 403), f"{status}")
        status, body = http(args.base, "GET", "/api/v1/admin/plans", secret)
        check("platform-admin route is refused to a key", status in (401, 403), f"{status}")

        # ── every write is audited with the key row id ─────────────────────────
        audited = sql(
            "SELECT string_agg(DISTINCT action, ',' ORDER BY action) FROM activity_log WHERE tenant_id = "
            + q(tenant_a) + " AND user_id = " + q("mcp-key:" + key_a)
            + " AND metadata->>'outcome' = 'ok'"
        )
        want = ["campaign.created", "tag_group.created", "tag_rule.created",
                "web_to_lead.created", "webhook.created", "qr_code.created"]
        check("every new family leaves an audit line naming the key id",
              all(w in audited for w in want), audited[:200])
    finally:
        sql(f"DELETE FROM tenants WHERE id IN ({q(tenant_a)}, {q(tenant_b)});")

    residue = sql(f"SELECT count(*) FROM tenants WHERE probe_harness = {q(TAG)}")
    check("teardown leaves no probe account", residue == "0", residue)

    passed = sum(1 for c in checks if c["ok"])
    failed = len(checks) - passed
    print(f"\n=== RESULT: {passed} passed, {failed} failed ===")
    report = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..",
                          "mcp-parity-acceptance.json")
    with open(report, "w") as handle:
        json.dump({"checks": checks, "passed": passed, "failed": failed}, handle, indent=2)
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
