#!/usr/bin/env python3
"""Live acceptance — plan §6 key-UI rules on FunnelSwift (card t_f39a88e9).

docs/fleet-per-account-keys-and-mcp-2026-11-25.md §6 (David, 2026-11-25): private-by-default keys,
View and Copy as SEPARATE controls, Copy fetches-and-clipboards WITHOUT rendering, the API Keys screen
is the rotate/revoke home, the MCP page's key card is FIRST with View+Copy ONLY (no rotate or
regenerate control, no handler), and every new account auto-mints ONE key labelled
`MCP — <account>` inside the account's own transaction.

Checks (one PASS/FAIL line each; exit 1 on any FAIL):
  1. fresh signup holds EXACTLY one live key labelled `MCP — <account>` (SQL)
  2. GET key list with the console session -> 200, masked only (prefix + …, no full secret in the body)
  3. Copy path: POST reveal (session, NO body) -> secret that answers tools/list 200 at POST /mcp
  4. View path: POST reveal -> 200 same secret AND exactly one new api_key_revealed audit row
  5. credential boundary: anon 401, an API KEY as the bearer 403, legacy {password} body 200 (ignored)
  6. rotate -> old secret tools/list 401, new secret 200, ONE live key, session still valid
  7. real-Chromium DOM: MCP key card is FIRST, has View+Copy, no rotate/regenerate control
  8. real-Chromium DOM: the MCP screen body carries no rotate/regenerate handler, Copy writes the
     clipboard and is never rendered, View renders with NO dialog
  9. real-Chromium DOM: the Settings API Keys screen keeps View+Copy+Regenerate+Revoke, stays
     masked, and its View is one click with no dialog
 10. teardown: probe account erased, residue 0

Never prints a full key — prefixes only.

Usage: python3 scripts/section6-key-ui-acceptance.py [base_url]
"""
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from urllib.parse import unquote, urlparse

BASE = (sys.argv[1] if len(sys.argv) > 1 else "https://app.funnelswift.net").rstrip("/")
APP_ENV = Path("/etc/swift/env/funnelswift.env")
HARNESS = "t_f39a88e9"

FAILED = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ((" — " + detail) if detail else ""))
    if not ok:
        FAILED.append(name)


def http(method, path, body=None, token=None, api_key=None, extra_headers=None, raw_body=False):
    url = BASE + path
    if body is None:
        data = b"" if raw_body else None
    else:
        data = json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    req.add_header("User-Agent", "swift-section6-acceptance")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    if api_key:
        req.add_header("Authorization", "Bearer " + api_key)
    for k, v in (extra_headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode(errors="replace")
            try:
                return r.status, (json.loads(raw) if raw else {})
            except json.JSONDecodeError:
                return r.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode(errors="replace")
        try:
            return e.code, (json.loads(raw) if raw else {})
        except json.JSONDecodeError:
            return e.code, raw
    except urllib.error.URLError as e:
        raise SystemExit(f"cannot reach {url}: {e}")


def _dsn():
    for line in APP_ENV.read_text().splitlines():
        if line.startswith("DATABASE_URL="):
            return line.split("=", 1)[1].strip().strip('"')
    raise SystemExit(f"no DATABASE_URL in {APP_ENV}")


def sql(query):
    u = urlparse(_dsn())
    db = unquote(u.path).lstrip("/") or "funnelswift"
    r = subprocess.run(
        ["docker", "exec", "-i", "-e", f"PGPASSWORD={unquote(u.password or '')}",
         "swift-postgres", "psql", "-U", unquote(u.username or "swift"), "-d", db,
         "-tAF", "|", "-v", "ON_ERROR_STOP=1", "-c", query],
        capture_output=True, text=True, timeout=60,
    )
    if r.returncode != 0:
        raise SystemExit(f"SQL failed: {r.stderr.strip()[:400]}\n--- {query[:200]}")
    return [line.split("|") for line in r.stdout.splitlines() if line.strip()]


def scalar(query):
    rows = sql(query)
    return rows[0][0] if rows else None


def mcp_tools_list(api_key):
    s, body = http("POST", "/mcp",
                   {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}, api_key=api_key)
    tools = []
    if s == 200 and isinstance(body, dict) and isinstance(body.get("result"), dict):
        tools = body["result"].get("tools") or []
    return s, tools


NODE_SCRIPT = r"""
const puppeteer = require('/usr/lib/node_modules/puppeteer');
(async () => {
  const token = process.env.FS_TOKEN;
  const secret = process.env.FS_SECRET;
  const browser = await puppeteer.launch({
    executablePath: '/usr/bin/chromium',
    headless: 'new',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
  });
  const dialogs = [];
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', e => errors.push(String(e)));
    page.on('dialog', async d => { dialogs.push({ type: d.type(), message: d.message() }); await d.dismiss(); });
    // Clipboard shim: proves Copy WRITES the clipboard through the page's own API without any
    // secret reaching the DOM. Re-installed on every navigation by evaluateOnNewDocument.
    await page.evaluateOnNewDocument(() => {
      window.__copied = null;
      try {
        Object.defineProperty(Navigator.prototype, 'clipboard', {
          configurable: true,
          value: { writeText: (t) => { window.__copied = String(t); return Promise.resolve(); } }
        });
      } catch (e) { window.__clipErr = String(e); }
    });

    await page.goto('https://app.funnelswift.net/', { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.evaluate((t) => localStorage.setItem('ws_fs_token', t), token);
    await page.goto('https://app.funnelswift.net/', { waitUntil: 'networkidle2', timeout: 30000 });

    // Open the MCP screen from the sidebar.
    const opened = await page.evaluate(() => {
      const a = document.querySelector('[data-tab="mcp"]');
      if (!a) return false;
      a.click();
      return true;
    });

    // Wait for the key card to render its row (it needs GET /api/v1/api-keys).
    let ready = false;
    for (let i = 0; i < 60; i++) {
      ready = await page.evaluate(() => {
        const card = document.getElementById('mcp-key-card');
        const list = document.getElementById('mcp-key-list');
        return !!(card && list && list.textContent.trim().length > 0);
      });
      if (ready) break;
      await new Promise(r => setTimeout(r, 250));
    }

    const dom = await page.evaluate(() => {
      const card = document.getElementById('mcp-key-card');
      const ct = document.getElementById('ct');
      const firstCard = ct ? ct.querySelector('.cc') : null;
      const controls = card ? [...card.querySelectorAll('a, button, [role=button]')] : [];
      const label = (el) => (el.textContent || '').trim();
      const banned = [];
      if (ct) {
        for (const el of [...ct.querySelectorAll('a, button, [role=button]')]) {
          if (/rotate|regenerate/i.test(label(el))) banned.push(label(el).slice(0, 60));
        }
      }
      const cardText = card ? (card.textContent || '') : '';
      return {
        firstCardIsKeyCard: !!card && firstCard === card,
        keyCardHasView: controls.some(el => label(el) === 'View'),
        keyCardHasCopy: controls.some(el => label(el) === 'Copy'),
        bannedControls: banned,
        cardShowsMaskedPrefix: /…|&hellip;/.test(cardText) || cardText.includes('\u2026'),
        cardHasFullSecret: /fs_[0-9a-f]{64}/.test(cardText),
      };
    });

    // View: one click, NO dialog, and the rendered value is the server's own key.
    const view = await page.evaluate(() => {
      const card = document.getElementById('mcp-key-card');
      const a = card ? [...card.querySelectorAll('a, button')].find(el => (el.textContent || '').trim() === 'View') : null;
      if (!a) return { clicked: false };
      a.click();
      return { clicked: true };
    });
    let viewVal = null;
    for (let i = 0; i < 40; i++) {
      viewVal = await page.evaluate(() => {
        const inp = document.querySelector('#mcp-key-out input');
        return inp ? inp.value : null;
      });
      if (viewVal) break;
      await new Promise(r => setTimeout(r, 250));
    }

    // Reload so the previous View cannot influence the Copy leg, then Copy from a clean DOM.
    await page.goto('https://app.funnelswift.net/', { waitUntil: 'networkidle2', timeout: 30000 });
    await page.evaluate(() => document.querySelector('[data-tab="mcp"]').click());
    for (let i = 0; i < 60; i++) {
      const ok = await page.evaluate(() => {
        const card = document.getElementById('mcp-key-card');
        const list = document.getElementById('mcp-key-list');
        return !!(card && list && list.textContent.trim().length > 0);
      });
      if (ok) break;
      await new Promise(r => setTimeout(r, 250));
    }
    const copyOutBefore = await page.evaluate(() => (document.getElementById('mcp-key-out') || {}).innerHTML || '');
    await page.evaluate(() => {
      const card = document.getElementById('mcp-key-card');
      const a = card ? [...card.querySelectorAll('a, button')].find(el => (el.textContent || '').trim() === 'Copy') : null;
      if (a) a.click();
    });
    for (let i = 0; i < 40; i++) {
      const got = await page.evaluate(() => window.__copied);
      if (got) break;
      await new Promise(r => setTimeout(r, 250));
    }
    const copy = await page.evaluate(() => ({
      copied: window.__copied,
      out: (document.getElementById('mcp-key-out') || {}).innerHTML || '',
      bodyHasSecret: document.body.innerHTML.includes(window.__copied || '###never###')
    }));

    // The MCP screen's own source slice — no rotate/regenerate handler may live there.
    const source = await page.evaluate(() => {
      const scripts = [...document.querySelectorAll('script')].map(s => s.textContent || '').join('\n');
      const start = scripts.indexOf('function LMCP(){');
      const end = scripts.indexOf('function MCPCLIENT(', start);
      if (start < 0 || end < 0 || end <= start) return null;
      return scripts.slice(start, end);
    });

    // ── Settings → API Keys: the ORIGINAL location keeps View, Copy, Regenerate (rotate), Revoke,
    // and its View is one click with no dialog too (plan §6.4/§6.2).
    await page.goto('https://app.funnelswift.net/', { waitUntil: 'networkidle2', timeout: 30000 });
    await page.evaluate(() => document.querySelector('[data-tab="settings"]').click());
    for (let i = 0; i < 60; i++) {
      const ok = await page.evaluate(() => {
        const box = document.getElementById('lm-ak-list');
        return !!(box && box.querySelector('[data-ak-view]'));
      });
      if (ok) break;
      await new Promise(r => setTimeout(r, 250));
    }
    const settings = await page.evaluate(() => {
      const box = document.getElementById('lm-ak-list');
      const texts = box ? [...box.querySelectorAll('a, button')].map(e => (e.textContent || '').trim()) : [];
      return {
        controls: texts,
        hasView: texts.includes('View'),
        hasCopy: texts.includes('Copy'),
        hasRotate: texts.includes('Regenerate'),
        hasRevoke: texts.includes('Revoke'),
        canvasHasSecret: /fs_[0-9a-f]{64}/.test(document.getElementById('lm-ak-list').textContent || ''),
      };
    });
    const dialogsBeforeSettings = dialogs.length;
    await page.evaluate(() => {
      const a = document.querySelector('#lm-ak-list [data-ak-view]');
      if (a) a.click();
    });
    let settingsViewVal = null;
    for (let i = 0; i < 40; i++) {
      settingsViewVal = await page.evaluate(() => {
        const inp = document.querySelector('#lm-ak-new input');
        return inp ? inp.value : null;
      });
      if (settingsViewVal) break;
      await new Promise(r => setTimeout(r, 250));
    }
    const settingsDialogs = dialogs.length - dialogsBeforeSettings;

    console.log(JSON.stringify({
      openedMcp: opened,
      ready,
      dom,
      viewClicked: view.clicked,
      viewRendersServerKey: !!viewVal && viewVal === secret,
      copyWroteClipboard: !!copy.copied && copy.copied === secret,
      copyRenderedNothing: (copyOutBefore || '').trim() === '' && (copy.out || '').trim() === '',
      copyLeakedToDom: copy.bodyHasSecret,
      mcpBodyHasRotateOrRegen: source === null ? null : /rotate|regenerate/i.test(source),
      settings,
      settingsViewRendersServerKey: !!settingsViewVal && settingsViewVal === secret,
      settingsViewDialogs: settingsDialogs,
      dialogs,
      pageErrors: errors,
    }));
  } finally {
    await browser.close();
  }
})().catch(e => { console.error('NODE-ERR', e && e.message); process.exit(1); });
"""


def dom_assertion(probe):
    env = dict(os.environ)
    env["FS_TOKEN"] = probe["token"]
    env["FS_SECRET"] = probe["secret"]
    node_file = Path("/root/.hermes/cache/scratch/fs-section6-dom.cjs")
    node_file.parent.mkdir(parents=True, exist_ok=True)
    node_file.write_text(NODE_SCRIPT)
    r = subprocess.run(["node", str(node_file)], capture_output=True, text=True, timeout=180, env=env)
    out = r.stdout.strip()
    if r.returncode != 0:
        return False, f"node rc={r.returncode} stderr={r.stderr.strip()[:400]}"
    for line in out.splitlines():
        if line.startswith("{"):
            try:
                return True, json.loads(line)
            except json.JSONDecodeError:
                return True, line
    return False, f"no JSON output: {out[:400]}"


def register_probe(tag):
    email = f"probe-section6-{tag}@example.invalid"
    password = "Section6Probe!2026"
    account_name = f"Section6 Probe {tag}"
    s, b = http("POST", "/api/v1/auth/register", {
        "name": f"Section6 Probe {tag}",
        "email": email,
        "password": password,
        "tenant_name": account_name,
    }, extra_headers={"X-Swift-Harness": HARNESS})
    if s not in (200, 201):
        raise SystemExit(f"probe signup failed: {s} {b}")
    b = b if isinstance(b, dict) else {}
    user = b.get("user") or {}
    return {
        "email": email,
        "password": password,
        "name": f"Section6 Probe {tag}",
        "account_name": account_name,
        "token": b["token"],
        "tenant_id": user.get("tenant_id"),
        "user_id": user.get("id"),
    }


def list_keys(p):
    s, b = http("GET", "/api/v1/api-keys", token=p["token"])
    keys = ((b or {}).get("api_keys") or []) if s == 200 else []
    return s, keys, b


def erase(p):
    s, _ = http("POST", "/api/v1/auth/erase-data",
                {"password": p["password"], "confirm": "DELETE"}, token=p["token"])
    return s


def main():
    tag = uuid.uuid4().hex[:8]
    a = register_probe(tag)
    print(f"probe account {a['account_name']}  tenant {a['tenant_id']}  user {a['user_id']}")

    # ── 1. exactly ONE live key labelled `MCP — <account>` ─────────────────────────────────────
    nkeys = scalar(f"SELECT COUNT(*) FROM api_keys WHERE tenant_id = '{a['tenant_id']}' AND revoked_at IS NULL")
    keyname = scalar(f"SELECT name FROM api_keys WHERE tenant_id = '{a['tenant_id']}' AND revoked_at IS NULL LIMIT 1")
    expect = f"MCP — {a['account_name']}"
    check("1 signup auto-minted EXACTLY one key labelled `MCP — <account>`",
          nkeys == "1" and keyname == expect,
          f"keys={nkeys} name={keyname!r} (expect {expect!r})")

    # ── 2. list is masked ───────────────────────────────────────────────────────────────────────
    s, keys, raw = list_keys(a)
    k = keys[0] if keys and isinstance(keys[0], dict) else None
    has_plaintext_field = any(("token" in kk) or ("key" in kk) or ("key_sealed" in kk) for kk in keys) \
        if isinstance(keys, list) else False
    import re as _re
    body_has_secret = bool(_re.search(r"fs_[0-9a-f]{64}", json.dumps(raw)))
    check("2 GET key list with the console session -> 200, masked only (no full secret)",
          s == 200 and len(keys) == 1 and not has_plaintext_field and not body_has_secret
          and bool(k and k.get("key_prefix")) and k.get("name") == expect,
          f"HTTP {s} keys={len(keys)} plaintext_field={has_plaintext_field} secret_in_body={body_has_secret}")

    # ── 3. Copy path: reveal (session, NO body) -> secret that opens the MCP door ──────────────
    s, body = http("POST", f"/api/v1/api-keys/{k['id']}/reveal", token=a["token"], raw_body=True)
    secret = body.get("token") if isinstance(body, dict) else None
    m3, tools3 = mcp_tools_list(secret) if isinstance(secret, str) else (None, [])
    check("3 Copy path: POST reveal (no body) -> secret that answers tools/list 200 at POST /mcp",
          s == 200 and isinstance(secret, str) and secret.startswith("fs_") and m3 == 200 and len(tools3) > 0,
          f"reveal HTTP {s} tools/list HTTP {m3} tools={len(tools3)}")
    a["secret"] = secret

    # ── 4. View path: reveal -> same secret + exactly ONE new audit row (who/key/when) ─────────
    before = int(scalar(f"SELECT COUNT(*) FROM activity_log WHERE action='api_key_revealed' AND entity_id='{k['id']}'") or 0)
    s4, body4 = http("POST", f"/api/v1/api-keys/{k['id']}/reveal", token=a["token"], raw_body=True)
    secret4 = body4.get("token") if isinstance(body4, dict) else None
    after = int(scalar(f"SELECT COUNT(*) FROM activity_log WHERE action='api_key_revealed' AND entity_id='{k['id']}'") or 0)
    who_when = sql(
        f"SELECT user_id::text, created_at::text FROM activity_log WHERE action='api_key_revealed' "
        f"AND entity_id='{k['id']}' ORDER BY created_at DESC LIMIT 1")
    who = who_when[0][0] if who_when else None
    when = who_when[0][1] if who_when and len(who_when[0]) > 1 else None
    check("4 View path: POST reveal -> 200 same secret AND exactly one new audit row (who/key/when)",
          s4 == 200 and secret4 == secret and (after - before) == 1 and who == a["user_id"] and bool(when),
          f"reveal HTTP {s4} same={secret4 == secret} audit delta={after - before} who={who} when={bool(when)}")

    # ── 5. credential boundary on BOTH doors: anon 401, key 403, legacy password body 200 ──────
    anon, _ = http("POST", f"/api/v1/api-keys/{k['id']}/reveal", raw_body=True)
    askey, _ = http("POST", f"/api/v1/api-keys/{k['id']}/reveal", api_key=secret, raw_body=True)
    legacy, body5 = http("POST", f"/api/v1/api-keys/{k['id']}/reveal",
                         {"password": "definitely-not-the-password"}, token=a["token"])
    legacy_tok = body5.get("token") if isinstance(body5, dict) else None
    # the fleet-standard twin (/api/personal-api-keys) is the same handler — prove it too
    ps, pbody = http("GET", "/api/personal-api-keys", token=a["token"])
    pkeys = (pbody.get("keys") if isinstance(pbody, dict) else None) or []
    p_anon, _ = http("POST", f"/api/personal-api-keys/{k['id']}/reveal", raw_body=True)
    p_key, _ = http("POST", f"/api/personal-api-keys/{k['id']}/reveal", api_key=secret, raw_body=True)
    p_ok, p_body = http("POST", f"/api/personal-api-keys/{k['id']}/reveal", token=a["token"], raw_body=True)
    p_tok = p_body.get("token") if isinstance(p_body, dict) else None
    twin_masked = bool(pkeys) and not _re.search(r"fs_[0-9a-f]{64}", json.dumps(pbody))
    check("5 credential boundary on both doors: anon 401, API key 403, legacy {password} 200 (ignored)",
          anon == 401 and askey == 403 and legacy == 200 and legacy_tok == secret
          and ps == 200 and twin_masked and p_anon == 401 and p_key == 403 and p_ok == 200 and p_tok == secret,
          f"v1[anon={anon} key={askey} legacy={legacy} same={legacy_tok == secret}] "
          f"personal[list={ps} masked={twin_masked} anon={p_anon} key={p_key} session={p_ok} same={p_tok == secret}]")

    # ── 6. rotate -> old 401, new 200, one live key, session untouched ─────────────────────────
    s6, body6 = http("POST", f"/api/v1/api-keys/{k['id']}/regenerate", {}, token=a["token"])
    new_secret = body6.get("token") if isinstance(body6, dict) else None
    m_old, _ = mcp_tools_list(secret) if isinstance(secret, str) else (None, [])
    m_new, t_new = mcp_tools_list(new_secret) if isinstance(new_secret, str) else (None, [])
    live_after = scalar(f"SELECT COUNT(*) FROM api_keys WHERE tenant_id = '{a['tenant_id']}' AND revoked_at IS NULL")
    me_status, _ = http("GET", "/api/v1/auth/me", token=a["token"])
    check("6 rotate -> old secret 401, new secret 200, ONE live key, session untouched",
          s6 in (200, 201) and isinstance(new_secret, str) and new_secret.startswith("fs_")
          and m_old == 401 and m_new == 200 and len(t_new) > 0 and live_after == "1" and me_status == 200,
          f"rotate HTTP {s6} old={m_old} new={m_new} tools={len(t_new)} live={live_after} session_me={me_status}")
    a["secret"] = new_secret

    # ── 7 + 8. real-Chromium DOM assertions on the served MCP screen ───────────────────────────
    ok, dom = dom_assertion(a)
    if ok and isinstance(dom, dict):
        d = dom.get("dom") or {}
        dialogs = dom.get("dialogs") or []
        passed7 = (dom.get("openedMcp") is True and dom.get("ready") is True
                   and d.get("firstCardIsKeyCard") is True
                   and d.get("keyCardHasView") is True
                   and d.get("keyCardHasCopy") is True
                   and (d.get("bannedControls") or []) == []
                   and d.get("cardShowsMaskedPrefix") is True
                   and d.get("cardHasFullSecret") is False)
        check("7 DOM: MCP key card FIRST, View+Copy present, masked, no rotate/regenerate control",
              passed7, json.dumps({kk: vv for kk, vv in dom.items() if kk != "pageErrors"})[:700])
        source_bad = dom.get("mcpBodyHasRotateOrRegen")
        passed8 = (source_bad is False
                   and dom.get("viewClicked") is True
                   and dom.get("viewRendersServerKey") is True
                   and len(dialogs) == 0
                   and dom.get("copyWroteClipboard") is True
                   and dom.get("copyRenderedNothing") is True
                   and dom.get("copyLeakedToDom") is False)
        check("8 DOM: no rotate handler in the MCP body; View one click no dialog; Copy writes clipboard, never rendered",
              passed8, json.dumps({
                  "mcpBodyHasRotateOrRegen": source_bad,
                  "viewRendersServerKey": dom.get("viewRendersServerKey"),
                  "dialogs": dialogs,
                  "copyWroteClipboard": dom.get("copyWroteClipboard"),
                  "copyRenderedNothing": dom.get("copyRenderedNothing"),
                  "copyLeakedToDom": dom.get("copyLeakedToDom"),
                  "pageErrors": dom.get("pageErrors"),
              })[:700])
        print("     DOM OUTPUT: " + json.dumps(dom)[:1800])
        st = dom.get("settings") or {}
        check("9 DOM: Settings API Keys keeps View+Copy+Regenerate+Revoke, list masked, View one click no dialog",
              st.get("hasView") is True and st.get("hasCopy") is True
              and st.get("hasRotate") is True and st.get("hasRevoke") is True
              and st.get("canvasHasSecret") is False
              and dom.get("settingsViewRendersServerKey") is True
              and (dom.get("settingsViewDialogs") or 0) == 0,
              json.dumps({"controls": st.get("controls"), "masked": st.get("canvasHasSecret") is False,
                          "view_renders_key": dom.get("settingsViewRendersServerKey"),
                          "view_dialogs": dom.get("settingsViewDialogs")})[:500])
    else:
        check("7 DOM: MCP key card FIRST, View+Copy present, masked, no rotate/regenerate control", False, str(dom)[:400])
        check("8 DOM: no rotate handler in the MCP body; View one click no dialog; Copy writes clipboard, never rendered", False, str(dom)[:400])
        check("9 DOM: Settings API Keys keeps View+Copy+Regenerate+Revoke, list masked, View one click no dialog", False, str(dom)[:400])

    # ── 10. teardown ────────────────────────────────────────────────────────────────────────────
    es = erase(a)
    if a["tenant_id"]:
        sql(f"DELETE FROM account_erasures WHERE tenant_ref = '{a['tenant_id']}';")
    residue = int(scalar(f"SELECT COUNT(*) FROM tenants WHERE id = '{a['tenant_id']}'") or 0)
    keys_left = int(scalar(f"SELECT COUNT(*) FROM api_keys WHERE tenant_id = '{a['tenant_id']}'") or 0)
    users_left = int(scalar(f"SELECT COUNT(*) FROM users WHERE tenant_id = '{a['tenant_id']}'") or 0)
    erasure_left = int(scalar(f"SELECT COUNT(*) FROM account_erasures WHERE tenant_ref = '{a['tenant_id']}'") or 0)
    check("10 probe account erased, residue 0",
          es == 200 and residue == 0 and keys_left == 0 and users_left == 0 and erasure_left == 0,
          f"erase={es} tenants={residue} keys={keys_left} users={users_left} erasure_rows={erasure_left}")

    print()
    if FAILED:
        print(f"{len(FAILED)} CHECK(S) FAILED: " + ", ".join(FAILED))
        return 1
    print("ALL 10 CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
