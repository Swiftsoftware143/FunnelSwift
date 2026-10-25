/* Pricing / Upgrade — loaded externally to avoid escaping issues */
/* 2026-10-19 (cron `default`): the Upgrade screen could not record anything. On this install no
   payment provider is configured, so POST /api/v1/checkout/create refuses every purchase (503
   `payment_provider_not_configured`); the screen said so in an alert that a reload erased, and its
   other failure arm told the customer to "open a support ticket" — a screen this app does not have.
   A customer who wanted to PAY left no trace the operator could act on.

   Now the ask is a row: `POST /api/v1/plan-requests` records it, `GET /api/v1/plan-requests` lists
   this workspace's asks with their live state, and the platform operator reads the same rows here
   (the admin arm of this screen) and moves the state on. The state shown is read from the server on
   every load, so a reload cannot lie about it. */
var PLANS = {};        // slug -> plan object, filled by renderUpgrade
var TERM = 'month';     // 'month' | 'year' — which term the cards are pricing
var LAST_PLANS = [];    // the catalogue as last rendered, so a term switch can repaint
var PREQS = [];        // this workspace's plan requests, newest first
var PREQ_OFFER = null; // {slug,name} — set when checkout refused, offered as a request button
var PREQ_MSG = null;   // {ok,text}

async function LPU(){
  $("pt").textContent="Upgrade Plan";
  $("ct").innerHTML='<div class="loading">Loading plans...</div>';
  try {
    var token = T || '';
    var resp = await fetch(window.location.origin+'/api/v1/plans', {headers:{Authorization:'Bearer '+token}});
    var plans = await resp.json();
    if(!Array.isArray(plans)) { $("ct").innerHTML='<div class="error">Failed to load plans.</div>'; return; }
    renderUpgrade(plans);
    loadRequests();
  } catch(e) { $("ct").innerHTML='<div class="error">'+e.message+'</div>'; }
}

function esc(v){
  return String(v == null ? '' : v).replace(/&/g,'&amp;').replace(/</g,'&lt;')
    .replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

var PREQ_LABEL = {
  open:      'Open — with our team',
  contacted: 'We have contacted you',
  done:      'Done — this workspace was moved',
  declined:  'Declined — see the note',
  withdrawn: 'Withdrawn by you'
};

function reqChip(status){
  var s = String(status || 'open');
  var bg = s === 'done' ? '#dcfce7' : (s === 'declined' ? '#fee2e2' : (s === 'contacted' ? '#dbeafe' : '#fef3c7'));
  var fg = s === 'done' ? '#166534' : (s === 'declined' ? '#991b1b' : (s === 'contacted' ? '#1e40af' : '#92400e'));
  return '<span style="background:'+bg+';color:'+fg+';padding:2px 8px;border-radius:10px;font-size:11px;font-weight:600">'
    + esc(PREQ_LABEL[s] || s) + '</span>';
}

async function loadRequests(){
  try {
    var token = T || '';
    var r = await fetch(window.location.origin+'/api/v1/plan-requests', {headers:{Authorization:'Bearer '+token}});
    if(r.ok){
      var d = await r.json();
      PREQS = Array.isArray(d && d.requests) ? d.requests : [];
    }
  } catch(e) { PREQS = []; }
  var box = document.getElementById('preq');
  if(box) box.innerHTML = renderRequests();
  if(window.U && window.U.is_admin) loadAdminRequests();
}

function renderRequests(){
  var h = '';
  if(PREQ_MSG){
    h += '<div style="background:'+(PREQ_MSG.ok?'#f0fdf4':'#fef2f2')+';border-left:4px solid '
      + (PREQ_MSG.ok?'#16a34a':'#dc2626') + ';padding:12px 14px;border-radius:6px;margin-bottom:14px;font-size:14px">'
      + esc(PREQ_MSG.text) + '</div>';
  }
  // The offer the checkout refusal raised: the customer's next step instead of a dead end.
  if(PREQ_OFFER && !PREQS.some(function(x){ return x.plan_slug === PREQ_OFFER.slug && (x.status === 'open' || x.status === 'contacted'); })){
    h += '<div data-plan-offer="'+esc(PREQ_OFFER.slug)+'" style="background:#fffbeb;border:1px solid #fde68a;border-radius:10px;padding:16px;margin-bottom:16px">'
      + '<p style="font-size:14px;color:#92400e;margin:0 0 10px">Upgrades are not open on this workspace yet — nothing was charged. '
      + 'We can move you onto the <strong>'+esc(PREQ_OFFER.name)+'</strong> plan ourselves and confirm here.</p>'
      + '<button data-request-plan="'+esc(PREQ_OFFER.slug)+'" onclick="requestPlan(\''+esc(PREQ_OFFER.slug)+'\')" '
      + 'style="padding:10px 16px;background:#d97706;color:#fff;border:none;border-radius:8px;font-size:14px;font-weight:600;cursor:pointer">'
      + 'Request the '+esc(PREQ_OFFER.name)+' plan</button></div>';
  }
  // This workspace's own asks, with the state the server holds.
  if(PREQS.length){
    h += '<div style="background:#fff;border:1px solid #e2e8f0;border-radius:10px;padding:16px;margin-bottom:16px">'
      + '<h4 style="font-size:15px;font-weight:700;margin:0 0 10px">Your plan requests</h4>';
    PREQS.forEach(function(q){
      h += '<div data-plan-request="'+esc(q.id)+'" style="padding:8px 0;border-bottom:1px solid #f1f5f9;font-size:13px">'
        + '<div style="display:flex;justify-content:space-between;gap:12px;align-items:center">'
        + '<div><strong>'+esc(q.plan_name || q.plan_slug)+'</strong>'
        + '<span style="color:#94a3b8"> · '+esc(String(q.created_at||'').slice(0,10))+'</span></div>'
        + '<div>'+reqChip(q.status)+'</div></div>';
      // The operator's own sentence, when there is one. A "declined" chip with no explanation is
      // the dead end this screen exists to remove, so the note is rendered where the state is.
      if(q.reply){
        h += '<div data-plan-request-reply style="margin-top:6px;background:#f8fafc;border-left:3px solid #6366f1;padding:8px 10px;border-radius:0 6px 6px 0;font-size:12px;color:#334155;white-space:pre-wrap">'
          + '<strong style="display:block;font-size:11px;color:#64748b;margin-bottom:2px">Note from our team</strong>'
          + esc(q.reply) + '</div>';
      }
      // An ask that is still unanswered can be taken back (2026-10-19): the row is what the operator
      // grants, so an ask the customer no longer wants must not stay in the queue waiting to be
      // acted on. Only the two OPEN states carry the control; an answered ask is a decision already
      // received, and the server refuses to rewrite it (409).
      if(q.status === 'open' || q.status === 'contacted'){
        h += '<div style="margin-top:6px"><button data-withdraw-plan-request="'+esc(q.id)+'" '
          + 'onclick="withdrawRequest(\''+esc(q.id)+'\')" '
          + 'style="padding:5px 10px;background:#fff;color:#64748b;border:1px solid #cbd5e1;border-radius:6px;font-size:12px;cursor:pointer">'
          + 'Withdraw this request</button></div>';
      }
      h += '</div>';
    });
    h += '<p style="font-size:11px;color:#94a3b8;margin:10px 0 0">Our team reads these. You will see the state change here once it is handled — or withdraw one yourself while it is still open.</p></div>';
  }
  return h;
}

/* Take back an ask that is still open. The server answers 200 for the change AND for an ask that was
   already withdrawn (the state the caller asked for is already true), and 409 once the team has
   answered — that sentence is shown as-is rather than guessed at here. */
async function withdrawRequest(id){
  var p = null;
  for(var i=0;i<PREQS.length;i++){ if(String(PREQS[i].id)===String(id)) p = PREQS[i]; }
  var name = p ? (p.plan_name || p.plan_slug) : 'that plan';
  if(!confirm('Withdraw your request for the ' + name + ' plan? Nothing has been charged, and you can ask again later.')) return;
  var token = T || '';
  PREQ_MSG = null;
  try {
    var r = await fetch(window.location.origin+'/api/v1/plan-requests/'+encodeURIComponent(id), {
      method:'DELETE',
      headers:{Authorization:'Bearer '+token}
    });
    var d = {};
    try { d = await r.json(); } catch(e) { d = {}; }
    if(r.ok){
      PREQ_MSG = {ok:true, text:'Request withdrawn. Nothing was charged — you can request the plan again at any time.'};
    } else if(r.status === 409){
      PREQ_MSG = {ok:false, text:(d && d.message) || 'That request has already been answered, so it cannot be withdrawn.'};
    } else {
      PREQ_MSG = {ok:false, text:(d && (d.message||d.error)) || ('The request could not be withdrawn (server answered '+r.status+').')};
    }
  } catch(e) {
    PREQ_MSG = {ok:false, text:'The request could not be withdrawn ('+(e && e.message ? e.message : e)+').'};
  }
  loadRequests();
}

async function requestPlan(slug){
  var p = PLANS[slug] || (PREQ_OFFER && PREQ_OFFER.slug === slug ? PREQ_OFFER : null);
  if(!p) return;
  var token = T || '';
  PREQ_MSG = null;
  try {
    var r = await fetch(window.location.origin+'/api/v1/plan-requests', {
      method:'POST',
      headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},
      body:JSON.stringify({plan_slug:slug})
    });
    var d = {};
    try { d = await r.json(); } catch(e) { d = {}; }
    if(!r.ok){
      PREQ_MSG = {ok:false, text:(d && (d.message||d.error)) || ('The request could not be sent (server answered '+r.status+'). Nothing was charged.')};
    } else if(d && d.status === 'existing'){
      PREQ_MSG = {ok:true, text:'Your request for the '+p.name+' plan is already with our team.'};
    } else {
      /* cron 2026-10-20 — the server now mails the filer a receipt. Say so only when the send
         actually happened: `acknowledged:false` means the row is on record and the mail did not go
         out (no provider configured, or a send failure), so promising a copy would be a lie.
         FAIL CLOSED on a missing field too (measured live 2026-10-20, cron): `=== false` only
         suppressed an EXPLICIT false, so a 200 whose body carried no `acknowledged` at all — an
         older server, a proxy, any caller of this endpoint that is not the current binary — made
         the console promise a copy nothing had queued. The promise needs a positive `true`, which
         is what ADASwift, IncentiveSwift and MissedCall already require. */
      var mailed = (d && d.acknowledged === true) ? ' We have emailed you a copy of this request.' : '';
      PREQ_MSG = {ok:true, text:'Request sent — our team will move this workspace to the '+esc(p.name)+' plan and confirm here. Nothing was charged.'+mailed};
    }
    PREQ_OFFER = null;
  } catch(e) {
    PREQ_MSG = {ok:false, text:'The request could not be sent ('+(e && e.message ? e.message : e)+'). Nothing was charged — please try again.'};
  }
  loadRequests();
}

/* The operator's queue. Same screen, admin arm: a workspace ask that nobody can see is the defect
   this whole screen was fixed for, so the list and the state buttons live beside the plans. */
async function loadAdminRequests(){
  var box = document.getElementById('preq-admin');
  if(!box) return;
  try {
    var token = T || '';
    var r = await fetch(window.location.origin+'/api/v1/admin/plan-requests', {headers:{Authorization:'Bearer '+token}});
    if(!r.ok){ box.innerHTML = ''; return; }
    var d = await r.json();
    var list = Array.isArray(d && d.requests) ? d.requests : [];
    if(!list.length){ box.innerHTML = ''; return; }
    var h = '<div data-admin-plan-requests style="background:#fff;border:2px solid #6366f1;border-radius:10px;padding:16px;margin-bottom:20px">'
      + '<h4 style="font-size:15px;font-weight:700;margin:0 0 4px">Plan requests from workspaces ('+list.length+')</h4>'
      + '<p style="font-size:11px;color:#64748b;margin:0 0 10px">Every ask a customer filed because the install has no payment provider. '
      + 'Move the state on here — the customer sees it on their own Upgrade screen.</p>';
    list.forEach(function(q){
      h += '<div data-admin-plan-request="'+esc(q.id)+'" style="padding:8px 0;border-bottom:1px solid #f1f5f9;font-size:13px">'
        + '<div style="display:flex;justify-content:space-between;gap:10px;align-items:center">'
        + '<div><strong>'+esc(q.tenant_name)+'</strong> · '+esc(q.plan_name || q.plan_slug)
        + '<div style="color:#94a3b8;font-size:11px">'+esc(q.requested_by||'')+' · '+esc(String(q.created_at||'').slice(0,16).replace('T',' '))+'</div></div>'
        + '<div style="display:flex;gap:6px;align-items:center">'+reqChip(q.status);
      ['contacted','done','declined'].forEach(function(s){
        if(q.status !== s){
          h += '<button data-set-status="'+esc(q.id)+'" data-status="'+s+'" onclick="setRequestStatus(\''+esc(q.id)+'\',\''+s+'\')" '
            + 'style="padding:4px 8px;font-size:11px;border:1px solid #cbd5e1;background:#f8fafc;border-radius:6px;cursor:pointer">'+s+'</button>';
        }
      });
      h += '</div></div>'
        + '<div style="display:flex;gap:6px;margin-top:6px">'
        + '<input data-reply-for="'+esc(q.id)+'" value="'+esc(q.reply||'')+'" placeholder="A note the customer reads on their Upgrade screen" '
        + 'style="flex:1;padding:6px 8px;font-size:12px;border:1px solid #cbd5e1;border-radius:6px">'
        + '<button onclick="saveRequestReply(\''+esc(q.id)+'\',\''+esc(q.status)+'\')" '
        + 'style="padding:4px 10px;font-size:11px;border:1px solid #6366f1;background:#eef2ff;color:#3730a3;border-radius:6px;cursor:pointer">Save note</button>'
        + '</div></div>';
    });
    h += '</div>';
    box.innerHTML = h;
  } catch(e) { box.innerHTML = ''; }
}

async function setRequestStatus(id, status){
  var token = T || '';
  try {
    var r = await fetch(window.location.origin+'/api/v1/admin/plan-requests/'+id, {
      method:'PUT',
      headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},
      body:JSON.stringify({status:status, reply:replyBoxValue(id)})
    });
    if(!r.ok){ var d = {}; try { d = await r.json(); } catch(e) {} alert((d && (d.message||d.error)) || ('Could not update ('+r.status+')')); return; }
  } catch(e) { alert('Could not update: '+e.message); return; }
  loadAdminRequests();
  loadRequests();
}

function replyBoxValue(id){
  var el = document.querySelector('[data-reply-for="'+id+'"]');
  return el ? String(el.value || '') : '';
}

/* Save the note on its own, leaving the state where it is: a "contacted" ask that needs one more
   sentence, or a note corrected after the fact, should not force a state change to be recorded. */
async function saveRequestReply(id, status){
  var token = T || '';
  try {
    var r = await fetch(window.location.origin+'/api/v1/admin/plan-requests/'+id, {
      method:'PUT',
      headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},
      body:JSON.stringify({status:status, reply:replyBoxValue(id)})
    });
    if(!r.ok){ var d = {}; try { d = await r.json(); } catch(e) {} alert((d && (d.message||d.error)) || ('Could not save the note ('+r.status+')')); return; }
  } catch(e) { alert('Could not save the note: '+e.message); return; }
  loadAdminRequests();
  loadRequests();
}

function termButton(v,label){
  var on = (TERM === v);
  return '<button type="button" data-term="'+v+'" onclick="setTerm(\''+v+'\')" style="padding:8px 14px;border-radius:999px;font-size:13px;font-weight:600;cursor:pointer;border:1px solid '+(on?'#6366f1':'#e2e8f0')+';background:'+(on?'#eef2ff':'#fff')+';color:'+(on?'#4338ca':'#475569')+'">'+label+'</button>';
}
function setTerm(v){ if(v !== TERM){ TERM = v; renderUpgrade(LAST_PLANS); } }

function renderUpgrade(plans){
  // Which plan is this workspace ACTUALLY on? `/auth/me` carries the plans row the account is
  // subscribed to (`current_plan`, with its real `slug`), and `plan_name` is the human label.
  // FunnelSwift sells TWO products (Capture and Kinetic) and BOTH have a free plan, so deciding by
  // "is it free" marked Capture Free AND Kinetic Free as "Current Plan" for a Capture account at
  // the same time — and hid the way up from the other one. Match the SLUG when the server gave us one,
  // the NAME otherwise; if neither resolves, leave every row actionable rather than mark a wrong one.
  PLANS = {};
  LAST_PLANS = plans || [];
  plans.forEach(function(p){ if(p && p.slug) PLANS[p.slug] = p; });

  var cp = (window.U && window.U.current_plan) || null;
  var curSlug = (cp && cp.slug) ? String(cp.slug) : '';
  var currentPlan = (cp && cp.name) || (window.U && (window.U.plan_name||window.U.plan)) || 'Free';
  var html = '<div style="max-width:900px;margin:0 auto"><h2 style="font-size:22px;font-weight:700">Upgrade Your Plan</h2>';
  html += '<p style="color:#64748b;margin-bottom:12px">Current: <strong>'+esc(currentPlan)+'</strong></p>';
  // SAY IT BEFORE THE CLICK (2026-10-25). Measured live on this installation: `payment_providers`
  // is empty and every `plans.purchase_url` is NULL, so `POST /api/v1/checkout/create` refuses every
  // paid tier with 503 `payment_provider_not_configured`. The screen used to let a customer pick a
  // tier, press Upgrade and only THEN learn the till is shut — one dead-end click on the surface
  // that takes money. The catalogue already carries `purchase_url` per plan, so the screen can tell
  // the difference: when NO paid tier carries a hosted payment page, it says so up front and the
  // cards below become a choice of plan rather than a promise of immediate purchase.
  var anyBuyable = (plans || []).some(function(p){
    return Number(p && p.price) > 0 && p.purchase_url && String(p.purchase_url).trim() !== '';
  });
  if (!anyBuyable && (plans || []).some(function(p){ return Number(p && p.price) > 0; })) {
    html += '<div data-payments-closed="1" style="margin:0 0 16px;padding:12px 14px;background:#fffbeb;'
          + 'border:1px solid #fde68a;border-radius:10px;color:#92400e;font-size:13.5px;line-height:1.45">'
          + '<strong>Payments are not open yet.</strong> Pick the plan you want below and press '
          + '<strong>Upgrade</strong> — your request is recorded against this workspace and the team '
          + 'moves you onto that plan as soon as billing opens. Nothing is charged today.'
          + '</div>';
  }
  // TERM SWITCH (2026-10-24). The public pricing page sells every paid tier with a yearly price
  // ("$9/month or $89 a year (two months free)") and `plans.annual_price` has carried those numbers
  // all along, but this screen only ever priced the monthly rate — so the yearly deal the site
  // advertises could not be bought from the surface where a customer actually buys.
  html += '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap;margin:0 0 16px">'
        + '<span style="font-size:13px;color:#64748b">Billing term:</span>'
        + termButton('month','Monthly')
        + termButton('year','Yearly \u2014 2 months free')
        + '</div>';
  html += '<div id="preq-admin"></div><div id="preq"></div>';
  html += '<div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:16px">';
  
  plans.forEach(function(p){
    var isCurrent = curSlug ? (p.slug === curSlug) : (p.name === currentPlan);
    var bg = isCurrent ? '#eef2ff' : '#fff';
    var border = isCurrent ? '2px solid #6366f1' : '1px solid #e2e8f0';
    html += '<div style="background:'+bg+';border:'+border+';border-radius:12px;padding:20px;display:flex;flex-direction:column">';
    html += '<div style="margin-bottom:16px"><h4 style="font-size:16px;font-weight:700;margin:0">'+esc(p.name)+'</h4>';
    html += '<span style="font-size:10px;color:#6366f1;background:#eef2ff;padding:2px 8px;border-radius:4px;font-weight:600">'+esc(String(p.side||'').toUpperCase())+'</span></div>';
    var _mo = Number(p.price === null || p.price === undefined ? 0 : p.price);
    var _yr = Number(p.annual_price === null || p.annual_price === undefined ? 0 : p.annual_price);
    if(TERM === 'year' && _yr > 0){
      // The saving is computed from the two figures in the same row, so a price change can never
      // leave a stale "2 months free" claim on the card.
      var _freeMonths = Math.round((_mo * 12 - _yr) / (_mo > 0 ? _mo : 1));
      var _saving = _freeMonths >= 1 ? (_freeMonths + ' month' + (_freeMonths === 1 ? '' : 's') + ' free') : ('$' + (_mo * 12 - _yr).toFixed(2) + ' off a year');
      html += '<div style="margin-bottom:2px"><span style="font-size:28px;font-weight:800">$'+esc(String(_yr))+'</span><span style="color:#64748b;font-size:13px">/year</span></div>';
      html += '<div style="margin-bottom:12px;font-size:12px;color:#16a34a;font-weight:600">$'+esc((_yr/12).toFixed(2))+'/mo \u2014 '+esc(_saving)+'</div>';
    } else {
      html += '<div style="margin-bottom:12px"><span style="font-size:28px;font-weight:800">$'+esc(p.price)+'</span><span style="color:#64748b;font-size:13px">/mo</span></div>';
      if(TERM === 'year' && _mo > 0){
        html += '<div style="margin:-8px 0 12px;font-size:12px;color:#64748b">Monthly only \u2014 this tier has no yearly price</div>';
      }
    }
    
    [
      {l:'Cards',v:p.max_cards},{l:'QR Codes',v:p.max_qr_codes},
      {l:'Forms',v:p.max_forms},{l:'Leads',v:p.max_leads},
      {l:'Card scans',v:p.max_ocr_scans},{l:'Tags',v:p.max_tags},
      {l:'Domains',v:p.max_custom_domains},{l:'Team',v:p.max_team_members}
    ].forEach(function(x){
      // `-1` is this app's storage convention for "unlimited" (plans.max_leads / max_tags carry it
      // on kinetic-pro). Only NULL used to render as the glyph, so a -1 printed as the literal text
      // "-1" beside a price on a buying surface (measured 2026-10-17, kanban t_8724c2bf).
      //
      // Card scans and Team joined this list because the customer's OWN dashboard names all EIGHT
      // allowances, and this comparison named only six — so the one limit that separates Capture
      // Free (10 scans) from Capture Starter (25 scans) was invisible on the very screen that sells
      // it, and a shopper could not check any Card-scans or Team figure they had just read at home
      // (measured live 2026-10-24: `plans` rows carry max_ocr_scans 10/25/NULL and max_team_members
      // 1 on every tier, while upgrade.js listed neither).
      //
      // A ZERO allowance is not a quantity either. The dashboard says "not on this plan" for one, so
      // a bare "0" here read as a count — on Capture Starter it made QR Codes look like a DOWNGRADE
      // from the free tier's 5. Same plan row, same read, so both surfaces say "Not included".
      var v = (x.v === null || x.v === undefined || Number(x.v) < 0) ? String.fromCharCode(8734)
            : (Number(x.v) === 0 ? 'Not included' : x.v);
      html += '<div style="display:flex;justify-content:space-between;font-size:11px;padding:3px 0;border-bottom:1px solid #f1f5f9"><span style="color:#64748b">'+x.l+'</span><span style="font-weight:600">'+v+'</span></div>';
    });
    
    html += '<div style="margin-top:8px;font-size:11px">';
    if(p.has_remove_branding) html += '<div style="color:#16a34a">&#10003; Remove Branding</div>';
    if(p.has_webhooks) html += '<div style="color:#16a34a">&#10003; Webhooks</div>';
    if(p.has_dual_routing) html += '<div style="color:#16a34a">&#10003; Dual Routing</div>';
    if(p.has_mini_funnels) html += '<div style="color:#16a34a">&#10003; Mini Funnels</div>';
    if(p.has_card_gating) html += '<div style="color:#16a34a">&#10003; Card Gating</div>';
    html += '</div>';
    
    html += '<div style="margin-top:auto;padding-top:12px">';
    // A $0 plan is not purchasable. `POST /api/v1/checkout/create` refuses one 400 "That plan costs
    // nothing — there is nothing to check out." (measured live 2026-10-15 on a real workspace token),
    // so on this install — which sells TWO products (Capture, Kinetic) and each has a free tier —
    // every non-current FREE tier rendered an "Upgrade" button whose only possible outcome was that
    // error. The button is replaced by a chip that says what is true, and each branch carries a
    // data- attribute so the rendered state can be asserted without reading button labels.
    // kanban t_ff1f0d6b.
    var price = Number(p.price === null || p.price === undefined ? 0 : p.price);
    if(isCurrent){
      html += '<div style="text-align:center;padding:10px;background:#6366f1;color:#fff;border-radius:8px;font-size:13px;font-weight:600">Current Plan</div>';
    } else if(price <= 0){
      html += '<div data-free-plan="'+esc(p.slug)+'" style="text-align:center;padding:10px;background:#eef2ff;color:#4338ca;border-radius:8px;font-size:13px;font-weight:600">Free plan — nothing to pay</div>';
    } else {
      html += '<button data-upgrade-plan="'+esc(p.slug)+'" onclick="upgradeTo(\''+esc(p.slug)+'\')" style="width:100%;padding:10px;background:#6366f1;color:#fff;border:none;border-radius:8px;font-size:14px;font-weight:600;cursor:pointer">Upgrade</button>';
    }
    html += '</div></div>';
  });
  
  html += '</div></div>';
  $("ct").innerHTML = html;
}

function upgradeTo(slug){
  // The confirmation names the PLAN and the AMOUNT, never the internal slug. Measured live
  // 2026-10-21 in Chromium on app.funnelswift.net: pressing Upgrade raised
  // "Upgrade to kinetic-pro plan?" — an internal identifier on the one screen where the price is
  // the whole question. The name/price come from the catalogue row this screen already rendered,
  // so the confirmation can never disagree with the card the customer just read.
  var planObj = PLANS[slug] || (PREQ_OFFER && PREQ_OFFER.slug === slug ? PREQ_OFFER : null);
  var label = (planObj && planObj.name) ? planObj.name : slug;
  var rawMo = planObj ? Number(planObj.price === null || planObj.price === undefined ? 0 : planObj.price) : 0;
  var rawYr = planObj ? Number(planObj.annual_price === null || planObj.annual_price === undefined ? 0 : planObj.annual_price) : 0;
  var useYear = (TERM === 'year' && rawYr > 0);
  var raw = useYear ? rawYr : rawMo;
  var amount = raw > 0 ? ' \u2014 $' + (raw % 1 === 0 ? String(raw) : raw.toFixed(2)) + (useYear ? '/year' : '/month') : '';
  if(!confirm('Upgrade this workspace to ' + label + amount + '?\n\nNothing is charged until the payment provider confirms the payment.')) return;
  try {
    var token = T || '';
    fetch(window.location.origin+'/api/v1/checkout/create', {
      method:'POST',
      headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},
      body:JSON.stringify({plan_slug:slug, billing_cycle:(useYear?'year':'month')})
    }).then(function(r){
      // The reply is NOT always JSON. A failed purchase can come back as a 5xx whose body the CDN
      // in front of this host has REPLACED (measured: the origin answered 502 application/json with
      // Stripe's own reason, while app.funnelswift.net answered 502 text/plain `error code: 502`), or
      // as a proxy/network error with no body at all. Reading it as TEXT and parsing defensively is
      // what stops a failed upgrade from showing the customer NOTHING at all.
      return r.text().then(function(t){
        var d = {};
        try { d = JSON.parse(t); } catch(e) { d = { _raw: String(t || '').slice(0, 200) }; }
        d._status = r.status;
        d._ok = r.ok;
        return d;
      });
    }).then(function(data){
      if(data && data.url) { window.location.href = data.url; return; }
      // A refusal that NAMES its cause (the provider's own words, e.g. Stripe's "Invalid API Key
      // provided") is the most useful thing we can show — but a buyer mid-purchase also needs to be
      // told the one thing they care about, and a refused /checkout/create never creates a session,
      // so no charge can have occurred on ANY of its failure arms.
      //
      // 2026-10-19: this used to END in an alert. An alert is not a record — a reload erased it, and
      // the operator never learned that someone tried to buy. The refusal now raises an inline
      // offer with ONE real next step (file the ask), which is written to `plan_requests` and read
      // back from the server. The customer's line still leads, and "nothing was charged" is still
      // said first, because that is what a buyer mid-purchase needs to know.
      if(data && data.message) {
        var operatorMsg = /provider|not configured/i.test(data.message);
        if(operatorMsg && PLANS[slug]){
          PREQ_OFFER = { slug: slug, name: PLANS[slug].name || slug };
          PREQ_MSG = { ok:true, text:'Upgrades are not open on this workspace yet — nothing was charged.' };
          loadRequests();
          window.scrollTo(0,0);
          // ONE PRESS FILES IT (cron 2026-10-20). Measured LIVE in Chromium on app.funnelswift.net
          // before this change: pressing Upgrade answered 503 /api/v1/checkout/create and only
          // RENDERED the offer card — GET /api/v1/plan-requests still returned 0 rows, so a buyer
          // who pressed once had asked for nothing. The ask is filed on that same press; the button
          // stays as the retry control (requestPlan's catch arm leaves PREQ_OFFER set on failure).
          requestPlan(slug);
          return;
        }
        var lead = operatorMsg
          ? 'Upgrades are not open on this workspace yet \u2014 nothing was charged.'
          : 'The upgrade could not be started \u2014 nothing was charged.';
        alert(lead + '\n\n' + data.message);
        return;
      }
      if(!data || !data._ok){
        alert('The upgrade could not be started (the server answered ' +
          ((data && data._status) || 'no reply') + '). Nothing was charged.\n\n' +
          'Please try again in a moment.');
        return;
      }
      alert('Upgrade request submitted. We will contact you.');
    }).catch(function(e){
      alert('The upgrade could not be started (' + ((e && e.message) ? e.message : e) +
        '). Nothing was charged — please check your connection and try again.');
    });
  } catch(e) { alert('Error: '+e.message); }
}
