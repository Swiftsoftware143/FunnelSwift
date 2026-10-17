/* Pricing / Upgrade — loaded externally to avoid escaping issues */
async function LPU(){
  $("pt").textContent="Upgrade Plan";
  $("ct").innerHTML='<div class="loading">Loading plans...</div>';
  try {
    var token = T || '';
    var resp = await fetch(window.location.origin+'/api/v1/plans', {headers:{Authorization:'Bearer '+token}});
    var plans = await resp.json();
    if(!Array.isArray(plans)) { $("ct").innerHTML='<div class="error">Failed to load plans.</div>'; return; }
    renderUpgrade(plans);
  } catch(e) { $("ct").innerHTML='<div class="error">'+e.message+'</div>'; }
}

function renderUpgrade(plans){
  // Which plan is this workspace ACTUALLY on? `/auth/me` carries the plans row the account is
  // subscribed to (`current_plan`, with its real `slug`), and `plan_name` is the human label.
  // FunnelSwift sells TWO products (Capture and Kinetic) and BOTH have a free plan, so deciding by
  // "is it free" marked Capture Free AND Kinetic Free as "Current Plan" for a Capture account at the
  // same time — and hid the way up from the other one. Match the SLUG when the server gave us one,
  // the NAME otherwise; if neither resolves, leave every row actionable rather than mark a wrong one.
  var cp = (window.U && window.U.current_plan) || null;
  var curSlug = (cp && cp.slug) ? String(cp.slug) : '';
  var currentPlan = (cp && cp.name) || (window.U && (window.U.plan_name||window.U.plan)) || 'Free';
  var html = '<div style="max-width:900px;margin:0 auto"><h2 style="font-size:22px;font-weight:700">Upgrade Your Plan</h2>';
  html += '<p style="color:#64748b;margin-bottom:20px">Current: <strong>'+currentPlan+'</strong></p>';
  html += '<div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:16px">';
  
  plans.forEach(function(p){
    var isCurrent = curSlug ? (p.slug === curSlug) : (p.name === currentPlan);
    var bg = isCurrent ? '#eef2ff' : '#fff';
    var border = isCurrent ? '2px solid #6366f1' : '1px solid #e2e8f0';
    html += '<div style="background:'+bg+';border:'+border+';border-radius:12px;padding:20px;display:flex;flex-direction:column">';
    html += '<div style="margin-bottom:16px"><h4 style="font-size:16px;font-weight:700;margin:0">'+p.name+'</h4>';
    html += '<span style="font-size:10px;color:#6366f1;background:#eef2ff;padding:2px 8px;border-radius:4px;font-weight:600">'+p.side.toUpperCase()+'</span></div>';
    html += '<div style="margin-bottom:12px"><span style="font-size:28px;font-weight:800">$'+p.price+'</span><span style="color:#64748b;font-size:13px">/mo</span></div>';
    
    [
      {l:'Cards',v:p.max_cards},{l:'QR Codes',v:p.max_qr_codes},
      {l:'Forms',v:p.max_forms},{l:'Leads',v:p.max_leads},
      {l:'Tags',v:p.max_tags},{l:'Domains',v:p.max_custom_domains}
    ].forEach(function(x){
      // `-1` is this app's storage convention for "unlimited" (plans.max_leads / max_tags carry it
      // on kinetic-pro). Only NULL used to render as the glyph, so a -1 printed as the literal text
      // "-1" beside a price on a buying surface (measured 2026-10-17, kanban t_8724c2bf).
      var v = (x.v === null || x.v === undefined || Number(x.v) < 0) ? String.fromCharCode(8734) : x.v;
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
      html += '<div data-free-plan="'+p.slug+'" style="text-align:center;padding:10px;background:#eef2ff;color:#4338ca;border-radius:8px;font-size:13px;font-weight:600">Free plan — nothing to pay</div>';
    } else {
      html += '<button data-upgrade-plan="'+p.slug+'" onclick="upgradeTo(\''+p.slug+'\')" style="width:100%;padding:10px;background:#6366f1;color:#fff;border:none;border-radius:8px;font-size:14px;font-weight:600;cursor:pointer">Upgrade</button>';
    }
    html += '</div></div>';
  });
  
  html += '</div></div>';
  $("ct").innerHTML = html;
}

function upgradeTo(slug){
  if(!confirm('Upgrade to '+slug+' plan?')) return;
  try {
    var token = T || '';
    fetch(window.location.origin+'/api/v1/checkout/create', {
      method:'POST',
      headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},
      body:JSON.stringify({plan_slug:slug})
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
      // 2026-10-15 (this arm used to open the alert with the server's sentence): on an install with
      // no payment provider configured the server answers the OPERATOR's instructions — measured
      // live in Chromium on app.funnelswift.net/dashboard (real mouse click on "Upgrade" -> 503
      // POST /api/v1/checkout/create -> the alert read "No payment provider is configured, so
      // checkout cannot work. The platform operator adds Stripe's keys in the admin panel …"). The
      // buyer is not the person who adds keys, and that sentence reads like something they broke.
      // Fleet convention (the sentence IncentiveSwift's Plan & Billing already ships): lead with
      // the customer's own line and the fact that nothing was charged, then the detail unchanged.
      // The operator keeps the whole reason — it is still printed above, in the same alert.
      if(data && data.message) {
        var operatorMsg = /provider|not configured/i.test(data.message);
        var lead = operatorMsg
          ? 'Upgrades are not open on this workspace yet \u2014 nothing was charged.'
          : 'The upgrade could not be started \u2014 nothing was charged.';
        alert(lead + '\n\n' + data.message);
        return;
      }
      if(!data || !data._ok){
        alert('The upgrade could not be started (the server answered ' +
          ((data && data._status) || 'no reply') + '). Nothing was charged.\n\n' +
          'Please try again in a moment. If it keeps happening, open a support ticket and quote ' +
          'that number.' + ((data && data._raw) ? '\n\nServer said: ' + data._raw : ''));
        return;
      }
      alert('Upgrade request submitted. We will contact you.');
    }).catch(function(e){
      alert('The upgrade could not be started (' + ((e && e.message) ? e.message : e) +
        '). Nothing was charged — please check your connection and try again.');
    });
  } catch(e) { alert('Error: '+e.message); }
}
