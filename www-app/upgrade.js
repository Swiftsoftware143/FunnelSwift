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
  var currentPlan = (window.U && (window.U.plan_name||window.U.plan)) ? (window.U.plan_name||window.U.plan) : 'Free';
  var html = '<div style="max-width:900px;margin:0 auto"><h2 style="font-size:22px;font-weight:700">Upgrade Your Plan</h2>';
  html += '<p style="color:#64748b;margin-bottom:20px">Current: <strong>'+currentPlan+'</strong></p>';
  html += '<div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:16px">';
  
  plans.forEach(function(p){
    var isCurrent = p.name === currentPlan || (currentPlan.toLowerCase().indexOf('free')>=0 && p.price===0);
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
      var v = (x.v !== null && x.v !== undefined) ? x.v : String.fromCharCode(8734);
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
    if(isCurrent){
      html += '<div style="text-align:center;padding:10px;background:#6366f1;color:#fff;border-radius:8px;font-size:13px;font-weight:600">Current Plan</div>';
    } else {
      html += '<button onclick="upgradeTo(\''+p.slug+'\')" style="width:100%;padding:10px;background:#6366f1;color:#fff;border:none;border-radius:8px;font-size:14px;font-weight:600;cursor:pointer">Upgrade</button>';
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
    fetch(window.location.origin+'/api/v1/checkout/create-session', {
      method:'POST',
      headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},
      body:JSON.stringify({plan_slug:slug})
    }).then(function(r){ return r.json(); }).then(function(data){
      if(data.url) window.location.href = data.url;
      else if(data.message) alert(data.message);
      else alert('Upgrade request submitted. We will contact you.');
    });
  } catch(e) { alert('Error: '+e.message); }
}
