/* kanban t_2d410c1f — the POST-PAYMENT LANDING (/checkout/success and /checkout/cancelled).
 *
 * `POST /api/v1/checkout/create` builds the Stripe session with a SERVER-MADE return URL
 * (src/handlers/checkout_handler.rs::create_stripe_session):
 *     success_url = {origin}/checkout/success?session_id={CHECKOUT_SESSION_ID}
 *     cancel_url  = {origin}/checkout/cancelled
 * and NOTHING served either path. Both fell through nginx `try_files $uri $uri/ /index.html`, so a
 * customer who had just paid was dropped on the dashboard with no confirmation that anything had
 * been bought (measured 2026-10-15: the served body carried 0 occurrences of `session_id`).
 *
 * The other half already existed and had ZERO callers: `GET /api/v1/checkout/session/:id`
 * (`checkout_handler::get_checkout_session_public`) is public BY DESIGN — id-scoped, presentation
 * safe fields only (plan name, amount, currency, status, created_at; never tenant_id/user_id/
 * provider_session_id/metadata) — and sits on the anonymous allowlist in `src/auth/route_policy.rs`
 * for exactly this case: "the buyer lands back from the payment provider with no JWT". This file is
 * its first caller.
 *
 * Two rules this page obeys:
 *   1. It says what the SERVER says. It never invents a status the API does not return, and an
 *      unresolvable id produces an honest "we could not find that checkout", never a fake receipt.
 *   2. It never claims the plan is already active. The plan change is applied by the
 *      SIGNATURE-VERIFIED Stripe webhook (POST /api/v1/webhooks/stripe, verified against the
 *      operator's stored webhook secret), which can land a moment after the redirect. So the page
 *      says the payment was received and that the plan updates on its own — and, when a session is
 *      present, quotes `/api/v1/auth/me` for the tier the workspace reads RIGHT NOW.
 */

(function () {
  'use strict';

  /* The path decides everything; every other URL boots the console exactly as before. */
  function returnKind() {
    var p = String(window.location.pathname || '').replace(/\/+$/, '');
    if (p === '/checkout/success') return 'success';
    if (p === '/checkout/cancelled' || p === '/checkout/canceled') return 'cancelled';
    return null;
  }

  /* `cs.amount` is numeric(12,2) handed over as text by the API ("9.00"). */
  function money(amount, currency) {
    var cur = String(currency || 'USD').toUpperCase();
    var n = Number(amount);
    var s = isFinite(n) ? n.toFixed(2) : String(amount === null || amount === undefined ? '' : amount);
    if (!s) return '—';
    return (cur === 'USD' ? '$' + s + ' USD' : s + ' ' + cur);
  }

  /* The provider's own status, in plain English. Anything this map does not know is printed
     VERBATIM in the card above — never translated into a claim. Each line names the row it is
     explaining, so the note can never be read as a different statement about the purchase. */
  function statusNote(status) {
    var s = String(status || '').toLowerCase();
    if (s === 'pending') return 'The server still reads this checkout as <strong>pending</strong>: the payment provider has not confirmed the payment to us yet. Your plan stays as it is until that confirmation arrives.';
    if (s === 'completed') return 'The server reads this checkout as <strong>completed</strong>: the payment provider has confirmed the payment to us.';
    if (s === 'expired') return 'The server reads this checkout as <strong>expired</strong>: it was never paid, so nothing was charged.';
    if (s === 'failed' || s === 'cancelled' || s === 'canceled') return 'The server reads this checkout as <strong>' + s + '</strong>: it did not complete, so nothing was charged.';
    return '';
  }

  function shell(inner) {
    return '<div style="max-width:620px;margin:0 auto">' + inner + '</div>';
  }

  /* The two lines under the table come from two DIFFERENT async sources — the public session
     lookup (the server's own status) and `/api/v1/auth/me` (the tier the workspace reads right
     now) — and either can land first. They each own a slot, so the later one can never erase the
     earlier one, which is what a single shared node did. */
  var notes = { status: '', tier: '' };

  function slot(id, html) {
    var el = document.getElementById(id);
    if (!el) return;
    if (html) { el.innerHTML = html; el.style.display = 'block'; }
    else { el.innerHTML = ''; el.style.display = 'none'; }
  }

  function paintNotes() {
    slot('fs-note', notes.status);
    slot('fs-tier', notes.tier);
  }

  function notesArea() {
    var box = 'background:#eef2ff;border-radius:10px;padding:12px 16px;margin:0 0 12px;font-size:13px;color:#3730a3';
    return '<div id="fs-note" style="display:none;' + box + '"></div>'
      + '<div id="fs-tier" style="display:none;' + box + '"></div>';
  }

  /* index.html calls this once `/auth/me` answers. Safe in either order: the value is kept and
     painted into whatever DOM exists when the landing has rendered. */
  window.fsCheckoutTier = function (html) { notes.tier = html || ''; paintNotes(); };

  function box(title, rows) {
    var h = '<div style="background:#fff;border:1px solid #e2e8f0;border-radius:12px;padding:18px 20px;margin:18px 0">';
    if (title) h += '<h3 style="font-size:13px;color:#1e1b4b;font-weight:700;margin:0 0 10px">' + title + '</h3>';
    rows.forEach(function (r) {
      h += '<div style="display:flex;justify-content:space-between;gap:16px;font-size:13px;padding:6px 0;border-bottom:1px solid #f1f5f9">'
        + '<span style="color:#64748b">' + r[0] + '</span>'
        + '<span style="font-weight:600;text-align:right;word-break:break-word">' + r[1] + '</span></div>';
    });
    return h + '</div>';
  }

  function btn(label, href, primary) {
    return '<a href="' + href + '" style="display:inline-block;padding:11px 22px;border-radius:8px;font-size:14px;font-weight:600;text-decoration:none;'
      + (primary ? 'background:#6366f1;color:#fff' : 'background:#eef2ff;color:#4338ca') + '">' + label + '</a>';
  }

  /* Where "carry on" points: a buyer with a session goes back into their workspace, a buyer
     without one (a receipt link followed on another device, or an expired session) signs in. */
  function workspaceHref(loggedIn) {
    return loggedIn ? '/app' : '/login.html';
  }

  function renderCancelled() {
    document.title = 'Checkout cancelled — FunnelSwift';
    var pt = document.getElementById('pt');
    if (pt) pt.textContent = 'Checkout cancelled';
    var ct = document.getElementById('ct');
    if (!ct) return;
    ct.innerHTML = shell(
      '<h2 style="font-size:22px;font-weight:700;color:#1e1b4b;margin-bottom:6px">Checkout cancelled</h2>'
      + '<p style="color:#475569;font-size:14px;line-height:1.6">You closed the payment page before paying, so '
      + '<strong>nothing was charged</strong> and your plan has not changed.</p>'
      + box('What now', [
        ['Your payment card', 'Not charged'],
        ['Your plan', 'Unchanged']
      ])
      + '<p style="color:#475569;font-size:13px;line-height:1.6">You can start the upgrade again whenever you like — '
      + 'open the <strong>Upgrade</strong> screen in your workspace, pick a plan and pay.</p>'
      + '<div style="margin-top:18px;display:flex;gap:10px;flex-wrap:wrap">'
      + btn(window.T ? 'Back to your workspace' : 'Sign in', workspaceHref(!!window.T), true)
      + '</div>'
      + '<p style="color:#94a3b8;font-size:12px;margin-top:16px">If a payment does show on your card, contact support with your '
      + 'provider receipt and we will put it right.</p>'
    );
  }

  function renderNotFound(sessionId, note) {
    var ct = document.getElementById('ct');
    if (!ct) return;
    var pt = document.getElementById('pt');
    if (pt) pt.textContent = 'Checkout not found';
    document.title = 'Checkout not found — FunnelSwift';
    var detail = note || 'No checkout session with that id.';
    ct.innerHTML = shell(
      '<h2 style="font-size:22px;font-weight:700;color:#1e1b4b;margin-bottom:6px">We could not find that checkout</h2>'
      + '<p style="color:#475569;font-size:14px;line-height:1.6">This page reads the checkout the payment provider sent you back with'
      + (sessionId ? ', and the reference in your link does not match a checkout on this account.' : ', and your link did not carry a checkout reference.')
      + '</p>'
      + box('This is what the server said', [['Detail', detail]])
      + '<p style="color:#475569;font-size:13px;line-height:1.6">If you have just paid, your payment provider receipt is authoritative: '
      + 'a plan purchase is applied by the provider\'s own confirmation, which reaches us separately from this page. '
      + 'Open your workspace to see your current plan, and contact support with that receipt if it has not changed.</p>'
      + '<div style="margin-top:18px;display:flex;gap:10px;flex-wrap:wrap">'
      + btn(window.T ? 'Open your workspace' : 'Sign in', workspaceHref(!!window.T), true)
      + '</div>'
    );
  }

  function renderSuccess(d, sessionId, loggedIn) {
    document.title = 'Payment received — FunnelSwift';
    var pt = document.getElementById('pt');
    if (pt) pt.textContent = 'Payment received';

    var status = String(d.status || '');
    var low = status.toLowerCase();
    var expired = (low === 'expired' || low === 'failed' || low === 'cancelled' || low === 'canceled');

    var amount = money(d.amount, d.currency);
    var planLabel = d.plan_name ? String(d.plan_name) : (d.purchasable_type ? String(d.purchasable_type) : 'Plan');

    var rows = [];
    rows.push(['Plan', planLabel]);
    rows.push(['Amount', amount]);
    rows.push(['Status', status || '—']);
    if (d.created_at) rows.push(['Started', String(d.created_at).replace('T', ' ').replace(/\..*$/, ' UTC')]);
    if (sessionId) rows.push(['Reference', '<span style="font-family:ui-monospace,Menlo,monospace;font-size:12px">' + sessionId + '</span>']);

    var head = expired
      ? '<h2 style="font-size:22px;font-weight:700;color:#b45309;margin-bottom:6px">This checkout did not complete</h2>'
        + '<p style="color:#475569;font-size:14px;line-height:1.6">The server has this checkout on file, but the payment provider\'s status for it is not a paid one.</p>'
      : '<h2 style="font-size:22px;font-weight:700;color:#1e1b4b;margin-bottom:6px">Payment received — thank you</h2>'
        + '<p style="color:#475569;font-size:14px;line-height:1.6">Your payment provider has taken you back to us with this checkout, and we have it on file.</p>';

    var body = shell(
      head
      + box('Your checkout', rows)
      + '<p style="color:#475569;font-size:13px;line-height:1.6">'
      + (expired
        ? 'Nothing further will be charged for this checkout.'
        : '<strong>Your plan updates on its own.</strong> The payment provider confirms the payment to us directly and the change is applied '
          + 'to the whole workspace automatically — usually within a minute or two. There is nothing you need to do.')
      + '</p>'
      + notesArea()
      + '<div style="margin-top:6px;display:flex;gap:10px;flex-wrap:wrap">'
      + (expired ? '' : btn(loggedIn ? 'Go to your workspace' : 'Sign in to your workspace', workspaceHref(loggedIn), true))
      + '<a href="/guide.html" style="display:inline-block;padding:11px 22px;border-radius:8px;font-size:14px;font-weight:600;text-decoration:none;background:#eef2ff;color:#4338ca">Read the user guide</a>'
      + '</div>'
      + '<p style="color:#94a3b8;font-size:12px;margin-top:16px">If your plan has not changed after a few minutes, contact support and quote the '
      + 'reference above together with your payment provider receipt.</p>'
    );

    var ct = document.getElementById('ct');
    if (ct) ct.innerHTML = body;

    notes.status = statusNote(status);
    paintNotes();
  }

  /* The loop a thank-you page cannot avoid: the session id comes from the provider's redirect, so
     it is read from the query string — and a missing/invalid one is a real state the page must
     answer honestly rather than dress up. */
  function renderSuccessOrNotFound(sessionId, loggedIn) {
    if (!sessionId) {
      return renderNotFound('', 'This page needs the checkout reference the payment provider adds to the link it sends you back with, and your link did not carry one.');
    }
    var url = window.location.origin + '/api/v1/checkout/session/' + encodeURIComponent(sessionId);
    return fetch(url, { headers: { Accept: 'application/json' } })
      .then(function (r) {
        return r.text().then(function (t) {
          var d = {};
          try { d = JSON.parse(t) || {}; } catch (e) { d = {}; }
          return { status: r.status, ok: r.ok, body: d, raw: t };
        });
      })
      .then(function (res) {
        if (res.ok && res.body && res.body.found) return renderSuccess(res.body, sessionId, loggedIn);
        renderNotFound(sessionId, (res.body && res.body.note) || ('The server answered ' + res.status + ' for that reference.'));
      })
      .catch(function (e) {
        var ct = document.getElementById('ct');
        if (!ct) return;
        ct.innerHTML = shell(
          '<h2 style="font-size:22px;font-weight:700;color:#1e1b4b;margin-bottom:6px">We could not check your checkout</h2>'
          + '<p style="color:#475569;font-size:14px;line-height:1.6">The page could not reach the server to look up this checkout '
          + '(' + (e && e.message ? e.message : e) + '). Your payment provider receipt is still authoritative.</p>'
          + '<div style="margin-top:18px">' + btn(loggedIn ? 'Open your workspace' : 'Sign in', workspaceHref(loggedIn), true) + '</div>'
        );
      });
  }

  /* Called by index.html instead of the console boot when this is a post-payment landing.
     `loggedIn` is the presence of a workspace session, never a claim about the purchase. */
  window.FSReturnPath = returnKind;
  window.renderCheckoutReturn = function (kind) {
    var loggedIn = !!window.T;

    /* A buyer who lands back with no session at all still gets the confirmation, so the console
       chrome that would only ever say "Loading..." (and the tab help this page has no tabs for)
       is hidden. With a session the buyer keeps the console around them and can carry on. */
    /* No console tab is open on a landing, so the sidebar must not claim Dashboard is the
       current screen (it is the boot default and it looked like the dashboard had loaded). */
    var links = document.querySelectorAll('.sidebar a');
    for (var i = 0; i < links.length; i++) links[i].classList.remove('active');

    if (!loggedIn) {
      ['sidebar', 'helpBtn', 'helpFab'].forEach(function (id) {
        var el = document.getElementById(id);
        if (el) el.style.display = 'none';
      });
      var hp = document.getElementById('helpPanel');
      if (hp) hp.style.display = 'none';
    }

    if (kind === 'cancelled') { renderCancelled(); return; }

    var sid = '';
    try { sid = new URLSearchParams(window.location.search).get('session_id') || ''; } catch (e) { sid = ''; }
    renderSuccessOrNotFound(sid.trim(), loggedIn);
  };
})();
