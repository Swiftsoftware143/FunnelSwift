
/* FunnelSwift Dashboard — All functions with DOM-based event handling */
/* No inline onclick escaping — zero-bug approach */

var T=localStorage.getItem("ws_fs_token")||"", U=null;

function api(p,o){
  o=o||{}; o.headers=o.headers||{};
  o.headers["Content-Type"]="application/json";
  if(T) o.headers["Authorization"]="Bearer "+T;
  return fetch(p,o).then(function(r){
    if(r.status===401){T="";localStorage.removeItem("ws_fs_token");window.location.href="/login";throw Error("auth")}
    if(!r.ok) return r.json().then(function(e){throw Error(e.error||e.message||"Error "+r.status)});
    return r.json();
  });
}

function X(s){return String(s||"").replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");}

function $(id){return document.getElementById(id);}

/* ═══ HARD-WIRED URL ROUTING MAP ═══
 * Prefix → Card Type → Branding CTA
 * DO NOT CHANGE without updating kinetic_handler.rs resolve_canonical_url + cta_for_prefix
 */
var CARD_ROUTES={
  k:{type:"Kinetic Card",cta:"Claim your free Kinetic Card →",desc:"Original/default card — all links, socials, media"},
  b:{type:"Bio Link",cta:"Claim your free Bio Link →",desc:"Single-page link collection"},
  c:{type:"Digital Business Card",cta:"Claim your free Digital Business Card →",desc:"Digital business card with contact info"},
  m:{type:"Micro Page",cta:"Claim your free Micro Page →",desc:"Compact landing/micro page"},
  f:{type:"Mini Funnel",cta:"Claim your free Mini Funnel →",desc:"Multi-step mini funnel"},
  h:{type:"Hero Page",cta:"Claim your free Hero Page →",desc:"Hero section with headline + CTA"}
};
/* ═══ END ROUTING MAP ═══ */
function TA(group){
  var a=document.getElementById("acc-"+group);
  if(a){ a.classList.toggle("open"); localStorage.setItem("fs_acc_"+group, a.classList.contains("open")?"1":"0"); }
}
function initAccordions(){
  ["affiliates","tags"].forEach(function(g){
    if(localStorage.getItem("fs_acc_"+g)==="1"){
      var a=document.getElementById("acc-"+g); if(a) a.classList.add("open");
    }
  });
}
/* ── Tab Switch (with accordion auto-open) ── */
function S(t){
  document.querySelectorAll(".sidebar a").forEach(function(a){a.classList.remove("active");});
  var l=document.querySelector('[data-tab="'+t+'"]');
  if(l) l.classList.add("active");
  /* Auto-open parent accordion */
  var accMap={stags:"tags",sgroups:"tags",affiliates:"affiliates",tiers:"affiliates",payouts:"affiliates"};
  var accGroup=accMap[t];
  if(accGroup){ var a=document.getElementById("acc-"+accGroup); if(a){a.classList.add("open"); localStorage.setItem("fs_acc_"+accGroup,"1");} }
  var ti={dashboard:"Dashboard",leads:"Leads",cards:"Kinetic Cards",tags:"Tags",tg:"Tag Groups",integrations:"Integrations",ap:"Affiliate Products",tenants:"Users",affiliates:"Affiliates",tiers:"Affiliate Tiers",payouts:"Affiliate Payouts",plans:"Plans",webhooks:"Webhooks",keys:"API Keys",domains:"Domain Settings",seo:"SEO Settings",stags:"System Tags",sgroups:"System Tag Groups",settings:"Settings","lead-stages":"Lead Stages"};
  $("pt").textContent=ti[t]||t;
  $("ct").innerHTML='<div class="loading">Loading...</div>';
  var L={dashboard:D,leads:LD,cards:LC,tags:LT,tg:LG,integrations:LI,apr:LPR,tenants:LN,affiliates:LA,ap:LP,tiers:LTR,payouts:LPA,plans:LPLL,webhooks:LW,keys:LK,domains:LDM,seo:LSEO,stags:LST,sgroups:LSG,settings:LM,"lead-stages":LLS};
  if(L[t]) L[t]();
}

/* Modal */
function O(title, bodyBuilder){
  $("mt").textContent=title;
  $("mb").innerHTML="";
  $("mf").innerHTML="";
  $("mm").className="msg";
  $("mod").classList.add("sh");
  bodyBuilder();
}
function C(){ $("mod").classList.remove("sh"); }

/* El: create element with properties */
function El(tag, props, children){
  var e=document.createElement(tag);
  if(props) for(var k in props){
    if(k==="class") e.className=props[k];
    else if(k==="onclick") e.onclick=props[k];
    else e.setAttribute(k, props[k]);
  }
  if(children){
    if(typeof children==="string") e.innerHTML=children;
    else if(Array.isArray(children)) children.forEach(function(c){e.appendChild(c);});
    else e.appendChild(children);
  }
  return e;
}

/* === AUTH === */
if(T){
  api("/api/v1/auth/me").then(function(u){
    if(u.error){window.location.href="/login"}
    else{
      U=u; $("ue").textContent=u.name||u.email;
      $("rb").innerHTML=u.is_admin?'<span class="badge bg">Admin</span>':'<span class="badge bb">User</span>';
      if(!u.is_admin){$("adm-sec").style.display="none";$("adm-links").style.display="none";}
      initAccordions(); S("dashboard");
    }
  });
}else{
  var p=new URLSearchParams(location.search);
  var tk=p.get("token");
  if(tk){T=tk;localStorage.setItem("ws_fs_token",tk);history.replaceState({},"","/app");location.reload();}
  else{window.location.href="/login";}
}

/* === DASHBOARD === */
function D(){
  api("/api/v1/dashboard/stats").then(function(d){
    var h="";
    h+='<div class="card"><h3>Total Leads</h3><div class="val">'+(d&&d.leads_total?d.leads_total:0)+'</div></div>';
    h+='<div class="card"><h3>Conv. Rate</h3><div class="val">'+(d&&d.conversion_rate?(d.conversion_rate*100).toFixed(1)+"%":"0%")+'</div></div>';
    h+='<div class="card"><h3>Cards</h3><div class="val">'+(d&&d.cards_total?d.cards_total:0)+'</div></div>';
    h+='<div class="card"><h3>Plans</h3><div class="val">'+(d&&d.plans_total?d.plans_total:0)+'</div></div>';
    $("sr").innerHTML=h;
  });
  api("/api/v1/dashboard/activity").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    var h='<div class="cc"><h3>Recent Activity</h3>';
    if(!a.length) h+='<p style="color:#64748b;font-size:13px">No recent activity.</p>';
    else{
      h+='<table><tr><th>Action</th><th>Description</th></tr>';
      a.slice(0,10).forEach(function(r){h+='<tr><td style="color:#6366f1;font-weight:600">'+X(r.action)+'</td><td>'+X(r.description)+'</td></tr>';});
      h+="</table>";
    }
    h+="</div>"; $("ct").innerHTML=h;
  });
}

/* === LEADS (DOM-based) === */function LD(){
  if(!U||!U.is_admin) return;
  api("/api/v1/leads").then(function(d){
    var a=(d&&d.data)?d.data:(Array.isArray(d)?d:[]);
    var total=d&&d.total?d.total:a.length;
    
    $("ct").innerHTML='<div class="cc"><h3>Leads ('+total+') <span id="ld-add-span"></span><span id="ld-stats"></span></h3><div id="ld-filters" style="display:flex;gap:10px;flex-wrap:wrap;margin-bottom:12px;align-items:center"></div><div id="ld-list"></div></div>';
    
    // Status badge summary  
    var counts={};
    a.forEach(function(l){var st=l.status||"new";counts[st]=(counts[st]||0)+1;});
    var sum="";
    Object.keys(counts).forEach(function(s){sum+='<span class="badge" style="margin-left:4px;font-size:10px;cursor:pointer" data-ld-filter="'+X(s)+'" onclick="LDf(\''+X(s)+'\')">'+X(s)+': '+counts[s]+'</span>';});
    $("ld-stats").innerHTML=sum;$("ld-add-span").appendChild(El("button",{"class":"btn btn-sm",style:"margin-left:8px",onclick:function(){MF()}},"+ Add Lead"));
    
    // Search bar
    var sf=$("ld-filters");
    var sb=El("input",{style:"padding:6px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px;width:200px",placeholder:"Search leads..."});
    sb.oninput=function(){LDf(null)};
    sf.appendChild(sb);
    sf.appendChild(El("button",{"class":"btn btn-sm btn-o",onclick:function(){sb.value="";LDf(null)}},"Clear"));
    
    var list=$("ld-list");
    if(!a.length){list.innerHTML='<p style="color:#64748b;padding:20px">No leads.</p>';return;}
    
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Name</th><th>Email</th><th>Phone</th><th>Source</th><th>Status</th><th></th></tr>';
    a.forEach(function(l){
      var tr=El("tr",{"class":"ld-row"});
      var name=(l.first_name||"")+" "+(l.last_name||"");
      if(!name.trim()) name=l.name||"Unnamed";
      var status=l.status||"new";
      var sc="#dbeafe"; var sc2="#1e40af";
      if(status==="Closed Won"||status==="qualified"){sc="#dcfce7";sc2="#166534";}
      else if(status==="Closed Lost"||status==="inactive"){sc="#fee2e2";sc2="#991b1b";}
      
      tr.setAttribute("data-search",(name+" "+l.email+" "+l.phone+" "+status+" "+l.source).toLowerCase());
      tr.setAttribute("data-status",status);
      tr.innerHTML='<td><strong>'+X(name)+'</strong>'+ (l.company?'<br><span style="font-size:10px;color:#94a3b8">'+X(l.company)+'</span>':'') +'</td><td>'+X(l.email||"-")+'</td><td>'+X(l.phone||"-")+'</td><td>'+X(l.source||"-")+'</td><td><span class="badge" style="background:'+sc+';color:'+sc2+';cursor:pointer" onclick="LDC(\''+l.id+'\',\''+X(status)+'\')">'+X(status)+'</span></td><td><button class="btn btn-sm" onclick="LDV(\''+l.id+'\')">View</button> <button class="btn btn-sm" onclick="MF(\''+l.id+'\')">Edit</button></td>';
      tbl.appendChild(tr);
    });
    list.appendChild(tbl);
  });
}
function LDf(q){
  var s=document.querySelector("#ld-filters input"); if(!s) return;
  var sq=s.value.toLowerCase();
  var af=document.querySelector(".badge[data-ld-filter].active")||null;
  document.querySelectorAll(".ld-row").forEach(function(r){
    var ds=r.getAttribute("data-search")||"";
    var st=r.getAttribute("data-status");
    var match=true;
    if(af&&st!==af.getAttribute("data-ld-filter")) match=false;
    else if(sq&&ds.indexOf(sq)<0) match=false;
    r.style.display=match?"":"none";
  });
}
function LDC(id,currentStatus){
  var stages=window._ld_stages||["New","Contacted","Qualified","Proposal"];
  if(!stages.length) stages=["New","Contacted","Qualified"];
  var st=El("select",{style:"width:100%;padding:6px"});
  stages.forEach(function(s){st.appendChild(El("option",{value:s,selected:s===currentStatus?"selected":null},s));});
  O("Change Status",function(){
    $("mb").innerHTML=""; 
    var d=El("div");
    d.appendChild(El("label",{style:"font-size:12px"},["New Status:"]));
    d.appendChild(st); $("mb").appendChild(d);
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){
      api("/api/v1/leads/"+id+"/status",{method:"PUT",body:JSON.stringify({status:st.value})}).then(function(){C();S("leads")}).catch(function(e){showMsg(e.message||"Failed","me")});
    }},"Save"));
  });
}
function LDV(id){
  api("/api/v1/leads").then(function(d){
    var a=(d&&d.data)?d.data:(Array.isArray(d)?d:[]);
    var l=a.find(function(x){return x.id===id});
    if(!l){showMsg("Lead not found","me");return;}
    O("Lead Detail",function(){
      $("mb").innerHTML="";
      var info=El("div",{style:"display:grid;grid-template-columns:1fr 1fr;gap:8px"});
      var fields=["first_name","last_name","name","email","phone","company","status","stage","source","notes","created_at"];
      var labels=["First Name","Last Name","Full Name","Email","Phone","Company","Status","Stage","Source","Notes","Created"];
      fields.forEach(function(k,i){
        var v=l[k]; if(v===null||v===undefined) v="-";
        if(k==="created_at"&&v!=="-") v=String(v).slice(0,19);
        info.appendChild(El("div",{},[El("span",{style:"font-size:10px;color:#64748b;display:block"},labels[i]),El("span",{style:"font-size:13px;font-weight:500"},String(v))]));
      });
      $("mb").appendChild(info);
      $("mf").appendChild(El("button",{"class":"btn",onclick:C},"Close"));
    });
  });
}
function MF(id){
  if(id) api("/api/v1/leads/"+id).then(function(l){RF(id,l)});
  else RF(null,{});
}
function RF(id,l){
  O(id?"Edit Lead":"Add Lead", function(){
    var cs=l.custom_fields&&typeof l.custom_fields==="object"?l.custom_fields:{};
    var mb=$("mb");
    var fn=El("input",{id:"fn",value:l.name||""});
    var fe=El("input",{id:"fe",value:l.email||""});
    var fp=El("input",{id:"fp",value:l.phone||""});
    var fw=El("input",{id:"fw",value:l.website||""});
    var fs=El("input",{id:"fs",value:l.source||""});
    var fst=El("select",{id:"fst"});
    ["new","contacted","qualified","converted","lost"].forEach(function(s){
      fst.appendChild(El("option",{value:s,selected:l.status===s?"selected":null},s));
    });
    var fnt=El("textarea",{id:"fnt",rows:"3"});
    fnt.textContent=l.notes||"";

    function addRow(k,v){
      var r=El("div",{"class":"cfr"});
      r.appendChild(El("input",{"class":"cfk",value:k||"",placeholder:"Name"}));
      r.appendChild(El("input",{"class":"cfv",value:String(v||""),placeholder:"Value"}));
      r.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){r.remove()}},"X"));
      return r;
    }
    var cfl=El("div",{id:"cfl"});
    Object.keys(cs).forEach(function(k){cfl.appendChild(addRow(k,cs[k]));});
    var addCFBtn=El("button",{"class":"btn btn-sm btn-o",style:"margin-top:4px",onclick:function(){cfl.appendChild(addRow("",""));}},"+ Add Field");

    mb.appendChild(ff("Name",fn));
    mb.appendChild(ff("Email",fe));
    mb.appendChild(ff("Phone",fp));
    mb.appendChild(ff("Website",fw));
    mb.appendChild(ff("Source",fs));
    mb.appendChild(ff("Status",fst));
    mb.appendChild(ff("Notes",fnt));
    // Tag assignment
    mb.appendChild(El("div",{},[El("label",{style:"font-size:12px;font-weight:600;color:#475569;display:block;margin-bottom:6px"},"Assign Tags"),El("div",{id:"tg-select",style:"display:flex;flex-wrap:wrap;gap:4px;max-height:120px;overflow-y:auto;padding:6px;border:1px solid #e2e8f0;border-radius:6px;min-height:36px"},El("span",{style:"color:#94a3b8;font-size:11px"},"Loading tags..."))]));
    api("/api/v1/tags").then(function(td){var all=Array.isArray(td)?td:(td&&td.data?td.data:[]);$("tg-select").innerHTML="";if(!all.length)$("tg-select").innerHTML='<span style="color:#94a3b8;font-size:11px">No tags available</span>';all.forEach(function(t){var tg=El("span",{style:"padding:3px 8px;border-radius:100px;font-size:10px;margin:2px;background:"+X(t.color||"#6366f1")+"20;color:"+X(t.color||"#6366f1")+";border:1px solid "+X(t.color||"#6366f1")+"40;cursor:pointer;user-select:none",onclick:function(){tg.classList.toggle("ts");if(tg.classList.contains("ts")){tg.style.background=t.color||"#6366f1";tg.style.color="#fff";tg.setAttribute("data-tid",t.id)}else{tg.style.background=(t.color||"#6366f1")+"20";tg.style.color=t.color||"#6366f1";tg.removeAttribute("data-tid")}}},t.name);$("tg-select").appendChild(tg)});});
    // Custom fields + Add button
    mb.appendChild(El("div",{style:"margin-top:12px"},[
      El("label",{style:"font-size:12px;font-weight:600;color:#475569;margin-bottom:8px;display:block"},"Custom Fields"),
      cfl, addCFBtn
    ]));

    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){SL(id)}},(id?"Save":"Create")));
  });
}
function ff(label, inp){
  var d=El("div",{"class":"fld"});
  d.appendChild(El("label",null,label));
  d.appendChild(inp);
  return d;
}
function SL(id){
  var d={name:$("fn").value, email:$("fe").value, phone:$("fp").value, website:$("fw").value, source:$("fs").value, status:$("fst").value, notes:$("fnt").value};
  var tids=[]; document.querySelectorAll(".ts[data-tid]").forEach(function(el){tids.push(el.getAttribute("data-tid"))});
  if(tids.length) d.tag_ids=tids;
  var cs={};
  var el=$("cfl")||document.createElement("div");
  el.querySelectorAll(".cfr").forEach(function(r){
    var k=r.querySelector(".cfk"), v=r.querySelector(".cfv");
    if(k&&v&&k.value.trim()) cs[k.value.trim()]=v.value;
  });
  d.custom_fields=cs;
  var m=id?"PUT":"POST", u=id?"/api/v1/leads/"+id:"/api/v1/leads";
  showMsg("Saving...","mi");
  api(u,{method:m,body:JSON.stringify(d)}).then(function(){C();S("leads")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}
function DL(id){if(confirm("Delete lead?")) api("/api/v1/leads/"+id,{method:"DELETE"}).then(function(){S("leads")});}

;(function(){var s=document.createElement("style");s.textContent=".lc-tab{display:inline-block;padding:8px 16px;font-size:13px;color:#64748b;text-decoration:none;border-bottom:2px solid transparent;cursor:pointer;font-weight:600}.lc-tab:hover{color:#6366f1}.lc-tab-a{color:#6366f1;border-bottom-color:#6366f1}.mc-tab{}.ts{outline:2px solid #6366f1 !important;outline-offset:1px}";document.head.appendChild(s);})();/* === KINETIC CARDS — Full: Templates, Preview, Themes, Domains === */
var KCB={cards:[],templates:[],themes:[],tab:"my"}; // Kinetic Card Binder state

function LC(){
  // Load everything in parallel
  Promise.all([
    api("/api/v1/kinetic/cards"),
    api("/api/v1/kinetic/templates"),
    api("/api/v1/kinetic/themes"),
    api("/api/v1/settings")
  ]).then(function(r){
    KCB.cards=(r[0]&&r[0].cards)?r[0].cards:(Array.isArray(r[0])?r[0]:[]);
    KCB.templates=(r[1]&&r[1].templates)?r[1].templates:(Array.isArray(r[1])?r[1]:[]);
    KCB.themes=Array.isArray(r[2])?r[2]:[];
    var s={}; r[3].forEach(function(x){s[x.key]=x.value;});
    KCB.subdomain=s.subdomain||""; KCB.custom_domain=s.custom_domain||"";
    LCR();
  });
}

function LCR(){
  var t=KCB.tab; // "my" or "tpl"
  var h='<div class="cc"><div id="lc-tabs" style="display:flex;gap:0;margin-bottom:16px;border-bottom:2px solid #e2e8f0">';
  h+='<a id="lc-tab-my" class="lc-tab '+(t==="my"?"lc-tab-a":"")+'" href="#">My Cards ('+KCB.cards.length+')</a>';
  h+='<a id="lc-tab-tpl" class="lc-tab '+(t==="tpl"?"lc-tab-a":"")+'" href="#">Template Library ('+KCB.templates.length+')</a>';
  h+='</div><div id="lc-body"></div></div>';
  $("ct").innerHTML=h;

  // Bind tab clicks
  $("lc-tab-my").onclick=function(e){e.preventDefault();KCB.tab="my";LCR();};
  $("lc-tab-tpl").onclick=function(e){e.preventDefault();KCB.tab="tpl";LCR();};

  if(t==="tpl") LCT(); else LCM();
}

/* === My Cards === */
function LCM(){
  var b=$("lc-body"); b.innerHTML="";
  var cards=KCB.cards;

  // Create button
  var bar=El("div",{style:"display:flex;gap:8px;margin-bottom:16px"});
  bar.appendChild(El("button",{"class":"btn",onclick:function(){MFC()}},"+ Create Card"));
  bar.appendChild(El("button",{"class":"btn btn-o",onclick:function(){KCB.tab="tpl";LCR();}},"Browse Templates"));
  b.appendChild(bar);

  if(!cards.length){b.appendChild(El("p",{style:"color:#64748b;text-align:center;padding:20px"},"No cards yet. Create one or pick a template."));}
  else{
    var grd=El("div",{"class":"grd"});
    cards.forEach(function(c){
      var it=El("div",{"class":"it"});
      // Type badge colors
      var tc=c.template_type||c.card_type||"default";
      var tcLabel={bio:"Bio Link",bio_link:"Bio Link",business:"Business Card",business_card:"Business Card",mini:"Mini Page",mini_page:"Mini Page",mini_funnel:"Mini Funnel",hero:"Hero",thank_you:"Thank You",default:"Custom"};
      var tcColor={bio:"#6366f1",bio_link:"#6366f1",business:"#0ea5e9",business_card:"#0ea5e9",mini:"#8b5cf6",mini_page:"#8b5cf6",mini_funnel:"#f59e0b",hero:"#10b981",thank_you:"#ec4899",default:"#64748b"};

      it.innerHTML='<div style="display:flex;justify-content:space-between;align-items:flex-start;margin-bottom:8px"><h4>'+X(c.title||"Untitled")+'</h4><span style="font-size:10px;padding:2px 6px;border-radius:4px;background:'+(tcColor[tc]||"#64748b")+'15;color:'+(tcColor[tc]||"#64748b")+';font-weight:600">'+(tcLabel[tc]||tc)+'</span></div>';
      it.innerHTML+='<p style="font-size:11px;color:#94a3b8">/'+(c.slug||"")+'</p>';
      if(c.bio) it.innerHTML+='<p style="font-size:12px;color:#64748b;margin-top:6px;line-height:1.4">'+X(c.bio.slice(0,80))+(c.bio.length>80?"...":"")+'</p>';
      it.innerHTML+='<p style="font-size:11px;color:#94a3b8;margin-top:4px">Views: '+(c.view_count||0)+'</p>';

      var act=El("div",{"class":"act"});
      act.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFC(c)}},"Edit"));
      act.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){window.open("https://kntcrd.com/k/"+(c.slug||"")+"?preview=1","_blank")}},"Preview"));
      act.appendChild(El("a",{"class":"btn btn-sm btn-o",style:"text-decoration:none",href:"https://kntcrd.com/k/"+(c.slug||""),target:"_blank"},"View"));
      act.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){DC(c.id)}},"Del"));
      it.appendChild(act);
      grd.appendChild(it);
    });
    b.appendChild(grd);
  }

  // Domain Settings
  b.appendChild(El("hr",{style:"margin:24px 0;border-color:#e2e8f0"}));
  b.appendChild(El("h4",{style:"font-size:14px;color:#1e1b4b;margin-bottom:6px"},"Your Card Domain"));
  b.appendChild(El("p",{style:"color:#64748b;font-size:12px;margin-bottom:12px"},"Your cards are published at <b>https://kntcrd.com/k/your-slug</b>"));
  var df=El("div",{id:"lc-df"});
  df.innerHTML='<div class="fld"><label>Your Subdomain</label><div style="display:flex;gap:6px;align-items:center;padding:8px 10px;background:#f8fafc;border:1px solid #e2e8f0;border-radius:6px"><b style="color:#1e1b4b;font-size:14px">'+X(KCB.subdomain||"ss")+'</b><span style="color:#94a3b8;font-size:13px">.kntcrd.com</span><span style="margin-left:auto;font-size:10px;color:#94a3b8;background:#f1f5f9;padding:2px 8px;border-radius:100px">LOCKED</span></div></div>';
  df.innerHTML+='<div class="fld"><label>Custom Domain (optional)</label><input id="lc-cd" value="'+X(KCB.custom_domain)+'" placeholder="mybrand.com" style="width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px"><p style="font-size:11px;color:#94a3b8;margin-top:4px">Point your domain to kntcrd.com with a CNAME record</p></div>';
  df.appendChild(El("button",{"class":"btn",onclick:function(){LCSD()}},"Save Custom Domain"));
  var dm=El("div",{id:"lc-dm","class":"msg"});
  b.appendChild(df);
  b.appendChild(dm);
}

/* === Template Library === */
function LCT(){
  var b=$("lc-body"); b.innerHTML="";
  var tpls=KCB.templates;

  // Filter bar
  var filter=El("div",{style:"display:flex;gap:8px;margin-bottom:16px;flex-wrap:wrap;align-items:center"});
  var allTypes=["all","bio_link","business_card","mini_page","mini_funnel","hero","thank_you"];
  var typeLabels={all:"All (30)",bio_link:"Bio Links (5)",business_card:"Business Cards (5)",mini_page:"Mini Pages (5)",mini_funnel:"Mini Funnels (5)",hero:"Heros (5)",thank_you:"Thank You (5)"};
  var vt=KCB.tplFilter||"all";

  allTypes.forEach(function(ty){
    var btn=El("button",{"class":"btn "+(vt===ty?"":"btn-o"),style:"padding:4px 10px;font-size:11px",onclick:function(){KCB.tplFilter=ty;LCT();}},typeLabels[ty]);
    filter.appendChild(btn);
  });
  b.appendChild(filter);

  // Filter templates
  var filtered=vt==="all"?tpls:tpls.filter(function(t){return t.type===vt;});

  // Search
  var search=El("input",{style:"width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px;margin-bottom:12px",placeholder:"Search templates..."});
  search.oninput=function(){
    var q=search.value.toLowerCase();
    document.querySelectorAll(".lc-tpl-card").forEach(function(el){
      var n=(el.getAttribute("data-name")||"").toLowerCase();
      var d=(el.getAttribute("data-desc")||"").toLowerCase();
      el.style.display=(n.indexOf(q)>=0||d.indexOf(q)>=0)?"":"none";
    });
  };
  b.appendChild(search);

  if(!filtered.length){b.appendChild(El("p",{style:"color:#64748b;text-align:center;padding:20px"},"No matching templates."));return;}

  var grd=El("div",{"class":"grd"});
  filtered.forEach(function(t){
    var tc=t.preview_colors||["#0f172a","#6366f1","#ffffff"];
    var card=El("div",{"class":"it lc-tpl-card",style:"border-left:3px solid "+tc[1]+";cursor:default"});
    card.setAttribute("data-name",t.name);
    card.setAttribute("data-desc",t.description||"");

    // Mini preview bar
    var preview=El("div",{style:"height:40px;border-radius:6px;margin-bottom:10px;background:linear-gradient(135deg,"+tc[0]+","+tc[1]+");display:flex;align-items:center;justify-content:center"});
    preview.appendChild(El("span",{style:"color:"+tc[2]+";font-size:10px;font-weight:700;letter-spacing:.1em"},(t.icon||"").toUpperCase()));
    card.appendChild(preview);

    card.innerHTML+='<div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:4px"><h4 style="font-size:13px">'+X(t.name)+'</h4><span style="font-size:10px;color:#94a3b8;background:#f1f5f9;padding:1px 6px;border-radius:4px">'+X(t.niche||"")+'</span></div>';
    card.innerHTML+='<p style="font-size:11px;color:#64748b;line-height:1.4;margin-bottom:8px">'+X(t.description||"")+'</p>';

    card.appendChild(El("button",{"class":"btn btn-sm",style:"width:100%",onclick:function(){MFC({template:t,slug:"",title:""})}},"Use Template"));
    grd.appendChild(card);
  });
  b.appendChild(grd);
}

/* === Card Editor — Side-by-Side with Live Preview === */
function MFC(data){
  var card=data&&data.id?data:null;
  var tpl=data&&data.template?data.template:null;

  var c=card||{card_type:"bio",accent_color:"#8b5cf6",bg_color:"#0f172a",text_color:"#ffffff",button_bg_color:"#1e293b",button_text_color:"#ffffff",template_type:"default",slug:"",title:""};
  if(tpl){
    c.card_type=tpl.type; c.template_type=tpl.type;
    if(tpl.preview_colors){c.accent_color=tpl.preview_colors[1];c.bg_color=tpl.preview_colors[0];}
    c.title=tpl.name||"";
    if(tpl.layout_blocks&&tpl.layout_blocks.length){
      var lb=tpl.layout_blocks[0];
      if(lb.catchphrase) c.tagline=lb.catchphrase;
      if(lb.title) c.title=lb.title;
      c.bio=lb.title||""; c.meta_description=lb.subtitle||lb.message||"";
    }
  }

  var mb=$("mb"); mb.innerHTML=""; $("mf").innerHTML=""; $("mm").className="msg";
  $("mt").textContent=card?"Edit Card":(tpl?"Create from "+tpl.name:"Create Card");
  $("mod").classList.add("sh");
  mb._cid=card?card.id:null;
  mb._slug_orig=card?card.slug:"";
  mb._card=c;

  // Make modal wider for split-pane
  document.querySelector(".modal").style.width="900px";

  // Split pane container
  var split=El("div",{style:"display:flex;gap:16px;min-height:420px"});

  // LEFT: Editor tabs (380px)
  var left=El("div",{style:"flex:0 0 380px;overflow-y:auto;max-height:70vh"});

  // RIGHT: Live Preview (flexible)
  var right=El("div",{style:"flex:1;position:sticky;top:0;align-self:flex-start"});
  var previewBox=El("div",{id:"mc-preview",style:"border-radius:8px;overflow:hidden;min-height:300px;text-align:center"});

  right.appendChild(El("p",{style:"font-size:11px;color:#94a3b8;margin-bottom:6px;font-weight:600;text-transform:uppercase;letter-spacing:.05em"},"Live Preview"));
  right.appendChild(previewBox);
  right.appendChild(El("p",{style:"font-size:10px;color:#94a3b8;margin-top:6px;text-align:center"},(c.title||"Untitled")+" at kntcrd.com/k/"+(c.slug||"your-slug")));

  split.appendChild(left);
  split.appendChild(right);
  mb.appendChild(split);

  // Tab navigation
  var tabBar=El("div",{style:"display:flex;gap:0;margin-bottom:14px;border-bottom:1px solid #e2e8f0"});
  var tabStyle="padding:6px 12px;font-size:11px;font-weight:600;cursor:pointer;border-bottom:2px solid transparent;color:#64748b";
  var tabActive="color:#6366f1;border-bottom-color:#6366f1";

  var tabs=["Info","Look & Feel","Social","Media"];
  var tabEls=[];
  tabs.forEach(function(n,i){
    var t=El("span",{style:tabStyle+(i===0?";"+tabActive:""),onclick:function(){showTab(i);}},n);
    tabEls.push(t);
    tabBar.appendChild(t);
  });
  left.appendChild(tabBar);

  function showTab(n){
    tabEls.forEach(function(t,i){t.style.cssText=tabStyle+(i===n?";"+tabActive:"");});
    document.querySelectorAll(".mc-tab-pane").forEach(function(el,i){el.style.display=i===n?"":"none";});
  }

  // TAB 0: Info
  var p0=El("div",{"class":"mc-tab-pane"});
  p0.appendChild(MF("Title","mc-title",c.title||"","My Card"));
  p0.appendChild(MF("Slug","mc-slug",c.slug||"","my-card"));
  var ts=El("select",{id:"mc-type"});
  [{v:"default",l:"Default"},{v:"bio_link",l:"Bio Link"},{v:"business_card",l:"Business Card"},{v:"mini_page",l:"Mini Page"},{v:"mini_funnel",l:"Mini Funnel"},{v:"hero",l:"Hero"},{v:"thank_you",l:"Thank You"}].forEach(function(o){
    ts.appendChild(El("option",{value:o.v,selected:(c.template_type||"default")===o.v?"selected":null},o.l));
  });
  p0.appendChild(MF("Template Type",ts));
  p0.appendChild(MF("Bio","mc-bio",c.bio||"","Bio / about section","textarea"));
  p0.appendChild(MF("Tagline","mc-tagline",c.tagline||"","Short headline"));
  p0.appendChild(MF("Meta Description","mc-meta",c.meta_description||"","SEO description","textarea"));
  left.appendChild(p0);

  // TAB 1: Look & Feel
  var p1=El("div",{"class":"mc-tab-pane",style:"display:none"});
  p1.appendChild(MF("Accent Color","mc-accent",c.accent_color||"#8b5cf6","","color"));
  p1.appendChild(MF("Background","mc-bg",c.bg_color||"#0f172a","","color"));
  p1.appendChild(MF("Text Color","mc-tc",c.text_color||"#ffffff","","color"));
  p1.appendChild(MF("Button BG","mc-bbg",c.button_bg_color||"#1e293b","","color"));
  p1.appendChild(MF("Button Text","mc-btc",c.button_text_color||"#ffffff","","color"));
  // Theme presets
  var themeDiv=El("div",{style:"margin-top:8px"});
  themeDiv.appendChild(El("label",{style:"font-size:11px;font-weight:600;color:#475569;display:block;margin-bottom:6px"},"Theme Presets"));
  var tg=El("div",{style:"display:grid;grid-template-columns:repeat(3,1fr);gap:6px"});
  KCB.themes.forEach(function(th){
    var thBtn=El("div",{className:"mc-theme-btn",style:"padding:6px;border-radius:6px;cursor:pointer;text-align:center;border:2px solid #e2e8f0;font-size:9px",onclick:function(){
      if(th.colors){
        $("mc-bg").value=th.colors.background||"#0f172a";
        $("mc-accent").value=th.colors.accent||"#6366f1";
        $("mc-tc").value=th.colors.text||"#ffffff";
        if(th.colors.button_bg) $("mc-bbg").value=th.colors.button_bg;
        if(th.colors.button_text) $("mc-btc").value=th.colors.button_text;
        updatePreview();
      }
      document.querySelectorAll(".mc-theme-btn").forEach(function(el){el.style.borderColor="#e2e8f0";});
      thBtn.style.borderColor="#6366f1";
    }});
    thBtn.innerHTML='<div style="height:20px;border-radius:3px;margin-bottom:3px;background:linear-gradient(135deg,'+(th.colors?th.colors.background||"#000":"#000")+','+(th.colors?th.colors.accent||"#fff":"#fff")+')"></div><span style="font-size:9px;color:#475569">'+X(th.name||"")+'</span>';
    tg.appendChild(thBtn);
  });
  themeDiv.appendChild(tg);
  p1.appendChild(themeDiv);
  left.appendChild(p1);

  // TAB 2: Social
  var p2=El("div",{"class":"mc-tab-pane",style:"display:none"});
  p2.appendChild(MF("LinkedIn","mc-linkedin",c.linkedin_url||"","https://linkedin.com/in/..."));
  p2.appendChild(MF("Twitter/X","mc-twitter",c.twitter_url||"","https://twitter.com/..."));
  p2.appendChild(MF("Instagram","mc-instagram",c.instagram_url||"","https://instagram.com/..."));
  p2.appendChild(MF("TikTok","mc-tiktok",c.tiktok_url||"","https://tiktok.com/@..."));
  p2.appendChild(MF("YouTube","mc-youtube",c.youtube_url||"","https://youtube.com/@..."));
  p2.appendChild(MF("Facebook","mc-facebook",c.facebook_url||"","https://facebook.com/..."));
  left.appendChild(p2);

  // TAB 3: Media
  var p3=El("div",{"class":"mc-tab-pane",style:"display:none"});
  p3.appendChild(MF("Avatar/Logo URL","mc-avatar",c.avatar_url||c.logo_url||"","https://..."));
  var vp=El("select",{id:"mc-vp"});
  [{v:"",l:"None"},{v:"youtube",l:"YouTube"},{v:"vimeo",l:"Vimeo"},{v:"loom",l:"Loom"},{v:"wistia",l:"Wistia"},{"v":"mp4",l:"MP4 URL"}].forEach(function(o){
    vp.appendChild(El("option",{value:o.v,selected:c.video_provider===o.v?"selected":null},o.l));
  });
  p3.appendChild(MF("Video Provider",vp));
  p3.appendChild(MF("Video ID/URL","mc-vid",c.video_id||"","Video ID or URL"));
  left.appendChild(p3);

  // Live Preview update function
  function updatePreview(){
    var t=getVal("mc-title")||"Untitled";
    var bio=getVal("mc-bio")||"";
    var tag=getVal("mc-tagline")||"";
    var bg=getVal("mc-bg")||"#0f172a";
    var ac=getVal("mc-accent")||"#8b5cf6";
    var tc=getVal("mc-tc")||"#ffffff";
    var bbg=getVal("mc-bbg")||"#1e293b";
    var btc=getVal("mc-btc")||"#ffffff";
    var av=getVal("mc-avatar");
    var li=getVal("mc-linkedin"), tw=getVal("mc-twitter"), ig=getVal("mc-instagram");
    var tt=getVal("mc-tiktok"), yt=getVal("mc-youtube"), fb=getVal("mc-facebook");
    var socials=[];
    if(li) socials.push({icon:"in",label:"LinkedIn",color:"#0077B5"});
    if(tw) socials.push({icon:"𝕏",label:"X",color:"#000"});
    if(ig) socials.push({icon:"📷",label:"IG",color:"#E4405F"});
    if(tt) socials.push({icon:"♪",label:"TikTok",color:"#000"});
    if(yt) socials.push({icon:"▶",label:"YouTube",color:"#FF0000"});
    if(fb) socials.push({icon:"f",label:"Facebook",color:"#1877F2"});

    var pb=$("mc-preview");
    pb.style.background=bg;
    pb.style.color=tc;
    pb.style.padding="30px 20px";

    var h='';
    if(av) h+='<div style="margin-bottom:16px"><img src="'+X(av)+'" style="width:80px;height:80px;border-radius:50%;object-fit:cover;border:3px solid '+ac+'" onerror="this.style.display=\'none\'"></div>';
    h+='<div style="font-size:24px;font-weight:800;margin-bottom:4px;color:'+tc+'">'+X(t)+'</div>';
    if(tag) h+='<div style="font-size:14px;color:'+ac+';margin-bottom:12px;font-weight:600">'+X(tag)+'</div>';
    if(bio) h+='<div style="font-size:14px;line-height:1.6;margin-bottom:20px;max-width:300px;margin-left:auto;margin-right:auto;color:'+tc+'">'+X(bio)+'</div>';

    // Action buttons
    if(bbg) h+='<div style="margin-bottom:20px"><span style="display:inline-block;padding:10px 28px;background:'+bbg+';color:'+btc+';border-radius:8px;font-size:14px;font-weight:700;border:none">Contact Me</span></div>';

    // Social icons
    if(socials.length){
      h+='<div style="display:flex;justify-content:center;gap:10px;flex-wrap:wrap">';
      socials.forEach(function(s){
        h+='<span style="display:inline-flex;align-items:center;gap:4px;padding:6px 12px;background:rgba(255,255,255,0.1);border-radius:20px;font-size:11px;color:'+tc+'">'+s.icon+' '+X(s.label)+'</span>';
      });
      h+='</div>';
    }

    pb.innerHTML=h;
  }

  // Bind all inputs to updatePreview
  setTimeout(function(){
    document.querySelectorAll("#mb input, #mb textarea, #mb select").forEach(function(el){
      el.addEventListener("input", updatePreview);
      el.addEventListener("change", updatePreview);
    });
    updatePreview();
  }, 100);

  // Footer
  $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:function(){document.querySelector(".modal").style.width="600px";C();}},"Cancel"));
  $("mf").appendChild(El("button",{"class":"btn",onclick:function(){SC2(mb._cid);}},(card?"Save":"Create")));
}

// Helper: create labeled form field
function MF(label, id, value, placeholder, type){
  var d=El("div",{"class":"fld"});
  d.appendChild(El("label",null,label));
  if(type==="textarea"){
    var ta=El("textarea",{id:id,rows:"3",placeholder:placeholder||""}); ta.textContent=value||"";
    d.appendChild(ta);
  }else if(type==="color"){
    d.appendChild(El("input",{id:id,type:"color",value:value||"#000000"}));
  }else if(type==="select"){
    // pass through — value is already a select element
    d.appendChild(value);
  }else{
    d.appendChild(El("input",{id:id,type:type||"text",value:value||"",placeholder:placeholder||""}));
  }
  return d;
}

function getVal(id){
  var el=$(id); return el?el.value:"";
}

function SC2(id){
  var d={
    title:getVal("mc-title"), slug:getVal("mc-slug"),
    template_type:getVal("mc-type")||"default",
    bio:getVal("mc-bio"), tagline:getVal("mc-tagline"),
    meta_description:getVal("mc-meta"),
    accent_color:getVal("mc-accent")||"#8b5cf6",
    bg_color:getVal("mc-bg")||"#0f172a",
    text_color:getVal("mc-tc")||"#ffffff",
    button_bg_color:getVal("mc-bbg")||"#1e293b",
    button_text_color:getVal("mc-btc")||"#ffffff",
    linkedin_url:getVal("mc-linkedin"), twitter_url:getVal("mc-twitter"),
    instagram_url:getVal("mc-instagram"), tiktok_url:getVal("mc-tiktok"),
    youtube_url:getVal("mc-youtube"), facebook_url:getVal("mc-facebook"),
    avatar_url:getVal("mc-avatar"),
    video_provider:getVal("mc-vp")||null, video_id:getVal("mc-vid")||null
  };
  if(!d.title.trim()){showMsg("Title is required","me");return;}
  if(!d.slug.trim()){showMsg("Slug is required","me");return;}

  var mu=id?"PUT":"POST", u=id?"/api/v1/kinetic/cards/"+id:"/api/v1/kinetic/cards";
  showMsg("Saving...","mi");
  api(u,{method:mu,body:JSON.stringify(d)}).then(function(r){
    document.querySelector(".modal").style.width="600px";
    C(); KCB.tab="my"; S("cards");
  }).catch(function(e){showMsg(e.message||"Save failed","me")});
}


function DC(id){
  if(confirm("Delete card permanently?"))
    api("/api/v1/kinetic/cards/"+id,{method:"DELETE"}).then(function(){KCB.tab="my";S("cards")});
}

function LCSD(){
  var m=$("lc-dm");
  if(!m) return;
  m.className="msg mi"; m.textContent="Saving...";
  api("/api/v1/settings",{method:"PUT",body:JSON.stringify({key:"custom_domain",value:($("lc-cd")||{}).value||""})})
  .then(function(){
    KCB.custom_domain=($("lc-cd")||{}).value||"";
    m.className="msg mk"; m.textContent="Custom domain saved!";
    setTimeout(function(){m.className="msg"},4000);
  }).catch(function(e){m.className="msg me";m.textContent=e.message;});
}


/* === TAGS (System/Custom) === */
function LT(){
  api("/api/v1/tags").then(function(d){
    var tags=Array.isArray(d)?d:((d&&d.data)?d.data:[]);
    var sys=[], cust=[];
    tags.forEach(function(t){if(t.is_system) sys.push(t);else cust.push(t);});
    $("ct").innerHTML='<div class="cc"><h3>Tags <span id="lt-add-btn"></span></h3><div id="lt-content"></div></div>';
    $("lt-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFT()}},"+ Custom Tag"));
    var ct=$("lt-content");

    // System tags
    ct.appendChild(El("p",{"class":"section-label"},"SYSTEM TAGS ("+sys.length+")"));
    if(!sys.length) ct.appendChild(El("p",{style:"color:#94a3b8;font-size:12px;padding:4px 12px"},"No system tags."));
    else{
      var div=El("div",{"class":"tags-inline"});
      sys.forEach(function(t){
        var chk=El("input",{type:"checkbox","class":"sys-tag-cb",value:t.id,style:"margin-right:4px;cursor:pointer"});
        var sp=El("span",{"class":"tag",style:"background:"+X(t.color||"#6366f1")+"20;color:"+X(t.color||"#6366f1")+";padding:4px 10px;border-radius:4px;font-size:12px;cursor:pointer",onclick:function(e){if(e.target.tagName!=="INPUT") MFT(t.id,true)}}, X(t.name)+' <span style="font-size:10px;opacity:.7">system</span>');
        var row=El("span",{style:"display:inline-flex;align-items:center;margin:2px 4px"},[chk,sp]);
        div.appendChild(row);
      });
      ct.appendChild(div);
    }

    // Custom tags
    ct.appendChild(El("p",{"class":"section-label"},"CUSTOM TAGS ("+cust.length+")"));
    if(!cust.length) ct.appendChild(El("p",{style:"color:#94a3b8;font-size:12px;padding:4px 12px"},"No custom tags yet. Create one above."));
    else{
      var tbl=El("table");
      tbl.innerHTML='<tr><th>Tag</th><th>Color</th><th>Group</th><th></th></tr>';
      cust.forEach(function(t){
        var tr=El("tr");
        tr.innerHTML='<td><span class="tag" style="background:'+X(t.color||"#6366f1")+'20;color:'+X(t.color||"#6366f1")+';padding:2px 8px;border-radius:4px;font-size:11px">'+X(t.name)+'</span></td><td><span style="display:inline-block;width:14px;height:14px;background:'+X(t.color||"#6366f1")+';border-radius:3px;vertical-align:middle;margin-right:4px"></span> '+X(t.color||"#6366f1")+'</td><td>'+X(t.group_id||"None")+'</td><td></td>';
        var td=tr.querySelectorAll("td")[3];
        td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFT(t.id)}},"Edit"));
        td.appendChild(document.createTextNode(" "));
        td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){DT(t.id)}},"Del"));
        tbl.appendChild(tr);
      });
      ct.appendChild(tbl);
    }
  });
}

/* Helper: show message in modal */
function showMsg(text, cls){
  var m=$("mm"); m.className="msg "+cls; m.textContent=text;
}

/* === TAG GROUPS (User) === */
function LG(){
  api("/api/v1/tag-groups").then(function(d){
    var groups=Array.isArray(d)?d:((d&&d.data)?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>My Tag Groups <span id="lg-add-btn"></span></h3><p style="color:#64748b;font-size:12px;margin-bottom:12px">Organize your custom tags into collapsible groups.</p><div id="lg-content"></div></div>';
    $("lg-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFG(null,false)}},"+ New Group"));
    var ct=$("lg-content");
    if(!groups.length){ct.appendChild(El("p",{style:"color:#94a3b8;padding:10px"},"No tag groups yet. Create one to organize your tags."));return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Group</th><th>Collapsible</th><th>Sort</th><th></th></tr>';
    groups.forEach(function(g){
      var tr=El("tr");
      tr.innerHTML='<td><strong>'+X(g.name)+'</strong></td><td>'+(g.is_collapsible?"Yes":"No")+'</td><td>'+(g.sort_order||0)+'</td><td></td>';
      var td=tr.querySelectorAll("td")[3];
      td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFG(g.id,false)}},"Edit"));
      td.appendChild(document.createTextNode(" "));
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){if(confirm("Delete group?")) api("/api/v1/tag-groups/"+g.id,{method:"DELETE"}).then(function(){S("tg")})}},"Del"));
      tbl.appendChild(tr);
    });
    ct.appendChild(tbl);
  });
}

/* === INTEGRATIONS (DOM-based) === */
function LI(){
  Promise.all([api("/api/v1/integration-targets"),api("/api/v1/affiliate-products"),api("/api/v1/api-keys")]).then(function(r){
    var tg=Array.isArray(r[0])?r[0]:(r[0]&&r[0].data?r[0].data:[]);
    var pr=Array.isArray(r[1])?r[1]:(r[1]&&r[1].data?r[1].data:[]);
    var ks=Array.isArray(r[2])?r[2]:(r[2]&&r[2].data?r[2].data:[]);

    $("ct").innerHTML='<div id="li-section"></div>';
    var root=$("li-section");

    // API Key
    var ak_div=El("div",{"class":"cc"},'<h3>Your API Key</h3><div id="li-apikey"></div>');
    var ak_body=ak_div.querySelector("#li-apikey");
    if(ks.length){
      var k=ks[0];
      ak_body.innerHTML='<p style="color:#64748b;font-size:13px;margin-bottom:8px">Use this to connect with other apps.</p><div style="background:#f8fafc;padding:12px;border-radius:6px;font-family:monospace;font-size:14px;display:flex;align-items:center;gap:8px"><code id="kd" style="flex:1">'+X(k.prefix||k.key||"")+'_'+Array(25).join("*")+'</code><span id="li-ak-btns"></span></div>';
      var btns=ak_body.querySelector("#li-ak-btns");
      btns.appendChild(El("button",{"class":"btn btn-sm",onclick:RK},"Show"));
      btns.appendChild(El("button",{"class":"btn btn-sm",onclick:CK},"Copy"));
      window._ak=k.key;
    }else{
      ak_body.innerHTML='<p style="color:#64748b;font-size:13px">No API key. </p>';
      ak_body.appendChild(El("button",{"class":"btn btn-sm btn-o",onclick:GK},"Generate"));
    }
    root.appendChild(ak_div);

    // SwiftSoftware Integrations
    var sw_div=El("div",{"class":"cc"},'<h3>SwiftSoftware Integrations</h3><div class="grd" id="li-sw-grd"></div>');
    var sw_grd=sw_div.querySelector("#li-sw-grd");
    [{n:"ADASwift",d:"ADA compliance scanning & reporting",u:"https://adaswift.com"},{n:"WorkflowSwift",d:"Automation workflows & n8n",u:"https://workflowswift.com"},{n:"CoreSwiftCRM",d:"CRM",u:"https://coreswiftcrm.com"},{n:"IncentiveSwift",d:"Incentive & rewards",u:"https://incentiveswift.com"},{n:"MissedCallRespondr",d:"Missed call automation",u:"https://missedcallrespondr.com"}].forEach(function(s){
      var it=El("div",{"class":"it"}); it.innerHTML='<h4>'+s.n+'</h4><p>'+s.d+'</p>';
      it.appendChild(El("a",{"class":"btn btn-sm btn-o",style:"margin-top:8px;text-decoration:none",href:s.u,target:"_blank"},"Connect"));
      sw_grd.appendChild(it);
    });
    root.appendChild(sw_div);

    // Affiliate Products
    var ap_div=El("div",{"class":"cc"},'<h3>Affiliate Products</h3><div class="grd" id="li-ap-grd"></div>');
    var ap_grd=ap_div.querySelector("#li-ap-grd");
    if(!pr.length) ap_grd.innerHTML='<p style="color:#64748b;font-size:13px;grid-column:1/-1">None yet.</p>';
    else pr.forEach(function(p){
      var it=El("div",{"class":"it"});
      it.innerHTML='<h4>'+X(p.name||"Product")+'</h4><p>Type: '+X(p.type||"")+' | Price: $'+(p.price||"0")+' | Commission: '+(p.default_commission_rate||"0")+'%</p>';
      it.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){CA(p.id)}},"Copy Affiliate Link"));
      ap_grd.appendChild(it);
    });
    root.appendChild(ap_div);

    // Integration Targets
    var tg_div=El("div",{"class":"cc"});
    tg_div.innerHTML='<h3>Integration Targets <span id="li-tg-btn"></span></h3><div id="li-tg-list"></div>';
    tg_div.querySelector("#li-tg-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFI()}},"+ Add Target"));
    var tg_list=tg_div.querySelector("#li-tg-list");
    if(!tg.length) tg_list.innerHTML='<p style="color:#64748b;padding:10px">No targets. Add webhook endpoints for third-party apps.</p>';
    else{
      var tbl=El("table");
      tbl.innerHTML='<tr><th>Name</th><th>URL</th><th>Events</th><th>Status</th><th></th></tr>';
      tg.forEach(function(t){
        var tr=El("tr");
        tr.innerHTML='<td><strong>'+X(t.name)+'</strong></td><td style="max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">'+X(t.webhook_url||"-")+'</td><td>'+(t.events?t.events.slice(0,3).map(X).join(", "):"-")+'</td><td><span class="badge '+(t.is_active?"bg":"br")+'">'+(t.is_active?"Active":"Off")+'</span></td><td></td>';
        var td=tr.querySelectorAll("td")[4];
        td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFI(t.id)}},"Edit"));
        td.appendChild(document.createTextNode(" "));
        td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){DI(t.id)}},"Del"));
        tbl.appendChild(tr);
      });
      tg_list.appendChild(tbl);
    }
    root.appendChild(tg_div);
  });
}
function RK(){if(window._ak) $("kd").textContent=window._ak;}
function CK(){var e=$("kd");if(e) navigator.clipboard.writeText(e.textContent).then(function(){e.textContent="Copied!";setTimeout(RK,2000);});}
function GK(){api("/api/v1/api-keys",{method:"POST",body:JSON.stringify({name:"Integration Key"})}).then(function(r){if(r&&r.key){window._ak=r.key;S("integrations");}});}
function CA(id){navigator.clipboard.writeText(window.location.origin+"/ref/"+id).then(function(){alert("Link copied!");});}

function MFI(id){
  if(id) api("/api/v1/integration-targets/"+id).then(function(t){RFI(id,t)});
  else RFI(null,{events:[],is_active:true});
}
function RFI(id,t){
  O(id?"Edit Target":"Add Integration Target", function(){
    var mb=$("mb");
    mb.appendChild(ff("Name", El("input",{id:"fn",value:t.name||""})));
    mb.appendChild(ff("Webhook URL", El("input",{id:"fu",value:t.webhook_url||""})));
    mb.appendChild(ff("API Key (optional)", El("input",{id:"fk",value:t.api_key||""})));
    mb.appendChild(ff("Events (comma-sep)", El("input",{id:"fv",value:(t.events||[]).join(", ")})));
    mb.appendChild(ff("Active", El("input",{id:"fa",type:"checkbox",checked:t.is_active!==false?"checked":null})));
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){SI(id)}},(id?"Save":"Create")));
  });
}
function SI(id){
  var ev=$("fv").value.split(",").map(function(s){return s.trim();}).filter(Boolean);
  var d={name:$("fn").value, webhook_url:$("fu").value, api_key:$("fk").value||null, events:ev, is_active:$("fa").checked};
  var m=id?"PUT":"POST", u=id?"/api/v1/integration-targets/"+id:"/api/v1/integration-targets";
  showMsg("Saving...","mi");
  api(u,{method:m,body:JSON.stringify(d)}).then(function(){C();S("integrations")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}
function DI(id){if(confirm("Delete target?")) api("/api/v1/integration-targets/"+id,{method:"DELETE"}).then(function(){S("integrations")});}

/* === AFFILIATE PRODUCTS (User) === */function LP(){
  if(!U||!U.is_admin) return;
  api("/api/v1/affiliate-products").then(function(d){
    var all=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Affiliate Products ('+all.length+') <span id="lap-add-btn"></span></h3><p style="color:#64748b;font-size:12px;margin-bottom:8px">Products affiliates can promote and earn commission on.</p><div id="lap-search-wrap" style="margin-bottom:12px"></div><div id="lap-list"></div></div>';
    $("lap-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){LPAF()}},"+ Add Product"));
    
    var search=El("input",{style:"width:100%;padding:6px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px",placeholder:"Search products by name, category..."});
    search.oninput=function(){
      var q=search.value.toLowerCase();
      document.querySelectorAll(".ap-row").forEach(function(r){
        var n=(r.getAttribute("data-search")||"").toLowerCase();
        r.style.display=n.indexOf(q)>=0?"":"none";
      });
    };
    $("lap-search-wrap").appendChild(search);
    
    if(!all.length){$("lap-list").innerHTML='<p style="color:#64748b;padding:20px">No products. Add one to let affiliates promote it.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Product</th><th>Category</th><th>Price</th><th>Commission</th><th>Active</th><th></th></tr>';
    all.forEach(function(p){
      var tr=El("tr",{"class":"ap-row"});
      tr.setAttribute("data-search",(p.name||"")+" "+(p.category_name||"")+" "+(p.description||""));
      tr.innerHTML='<td><strong>'+X(p.name||"Product")+'</strong>'+ (p.description?'<br><span style="font-size:10px;color:#94a3b8">'+X(p.description)+'</span>':'') +'</td><td>'+X(p.category_name||"-")+'</td><td>$'+(p.price||"0")+'</td><td>'+(p.default_commission_rate?p.default_commission_rate+"%":"-")+'</td><td><span class="badge" style="background:'+(p.is_active?"#dcfce7;color:#166534":"#fee2e2;color:#991b1b")+'">'+(p.is_active?"Active":"Inactive")+'</span></td><td></td>';
      var td=tr.querySelectorAll("td")[5];
      td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){LPAF(p.id)}},"Edit"));
      td.appendChild(document.createTextNode(" "));
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){if(confirm("Delete \""+X(p.name)+"\"?")) api("/api/v1/affiliate-products/"+p.id,{method:"DELETE"}).then(function(){S("ap")});}},"Del"));
      tbl.appendChild(tr);
    });
    $("lap-list").appendChild(tbl);
  });
}

/* Affiliate Product Form (Admin CRUD) */
function LPAF(id){
  if(id) api("/api/v1/affiliate-products").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    var p=a.find(function(x){return x.id===id});
    LPAFS(p||{id:id});
  });
  else LPAFS({is_active:true,default_commission_rate:10});
}
function LPAFS(p){
  O(p.id?"Edit Product":"Add Product",function(){
    var mb=$("mb"); mb.innerHTML=""; $("mf").innerHTML="";
    mb.appendChild(ff("Name", El("input",{id:"fan",value:p.name||""})));
    mb.appendChild(ff("Category", El("input",{id:"fac",value:p.category_name||""})));
    mb.appendChild(ff("Price ($)", El("input",{id:"fap",type:"number",value:p.price||""})));
    mb.appendChild(ff("Commission %", El("input",{id:"far",type:"number",value:p.default_commission_rate||""})));
    mb.appendChild(ff("URL", El("input",{id:"fau",value:p.url||"",placeholder:"Product page URL"})));
    mb.appendChild(ff("Description", El("textarea",{id:"fad",value:p.description||"",style:"height:60px"})));
    var ac=ff("Active", El("input",{id:"faa",type:"checkbox",checked:p.is_active?"checked":null}));
    ac.querySelector("label").style.display="inline-flex";ac.querySelector("label").style.gap="6px";
    ac.querySelector("input").style.width="auto";
    mb.appendChild(ac);
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){
      var d={name:$("fan").value,category_name:$("fac").value,price:parseFloat($("fap").value)||0,default_commission_rate:parseFloat($("far").value)||0,url:$("fau").value,description:$("fad").value,is_active:$("faa").checked};
      if(!d.name.trim()){showMsg("Name is required","me");return;}
      showMsg("Saving...","mi");
      var m=p.id?"PUT":"POST", u=p.id?"/api/v1/affiliate-products/"+p.id:"/api/v1/affiliate-products";
      api(u,{method:m,body:JSON.stringify(d)}).then(function(){C();S("ap")}).catch(function(e){showMsg(e.message||"Save failed","me")});
    }},"Save"));
  });
}

/* Affiliate Products (User — read-only catalog) */
function LPR(){
  api("/api/v1/affiliate-products").then(function(d){
    var all=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Affiliate Products ('+all.length+')</h3><p style="color:#64748b;font-size:12px;margin-bottom:12px">Products you can promote as an affiliate. Contact admin about adding new products.</p><div id="lpr-list"></div></div>';
    if(!all.length){$("lpr-list").innerHTML='<p style="color:#64748b;padding:20px;text-align:center">No products available yet.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Product</th><th>Category</th><th>Price</th><th>Commission</th></tr>';
    all.forEach(function(p){
      var tr=El("tr");
      tr.innerHTML='<td><strong>'+X(p.name||"Product")+'</strong>'+ (p.description?'<br><span style="font-size:10px;color:#94a3b8">'+X(p.description)+'</span>':'') +'</td><td>'+X(p.category_name||"-")+'</td><td>$'+(p.price||"0")+'</td><td>'+(p.default_commission_rate?p.default_commission_rate+"%":"-")+'</td>';
      tbl.appendChild(tr);
    });
    $("lpr-list").appendChild(tbl);
  });
}
function LDM(){
  api("/api/v1/settings").then(function(d){
    var s={}; d.forEach(function(x){s[x.key]=x.value;});
    var mb=$("ct"); mb.innerHTML='<div class="cc"><h3>Domain Settings</h3><p style="color:#64748b;font-size:13px;margin-bottom:12px">Subdomain is tenant-locked. Configure your custom domain below.</p><div id="ds-form"></div><div id="ds-msg" class="msg"></div><div id="ds-btns" style="margin-top:12px"></div></div>';
    var f=$("ds-form");
    f.innerHTML='<div class="fld"><label>Subdomain (tenant)</label><div style="display:flex;gap:6px;align-items:center;margin-bottom:16px;padding:8px 10px;background:#f8fafc;border:1px solid #e2e8f0;border-radius:6px"><b style="color:#1e1b4b;font-size:14px">'+X(s.subdomain||"Not set")+'</b><span style="color:#94a3b8;font-size:13px">.kntcrd.com</span></div></div>';
    f.innerHTML+='<div class="fld"><label>Custom Domain (optional)</label><input id="ds-cd" value="'+X(s.custom_domain||"")+'" placeholder="mybrand.com" style="width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px;margin-bottom:4px"><p style="font-size:11px;color:#94a3b8;margin-top:4px">Point your domain to kntcrd.com with a CNAME record</p></div>';
    $("ds-btns").appendChild(El("button",{"class":"btn",onclick:SD},"Save Custom Domain"));
  });
}
function SD(){
  var m=$("ds-msg"); m.className="msg mi"; m.textContent="Saving...";
  api("/api/v1/settings",{method:"PUT",body:JSON.stringify({key:"custom_domain",value:$("ds-cd").value.trim()})})
    .then(function(){m.className="msg mk";m.textContent="Custom domain saved!";setTimeout(function(){m.className="msg"},3000);})
    .catch(function(e){m.className="msg me";m.textContent=e.message;});
}

/* === TENANTS (Full CRUD, DOM-based) === */
function LN(){
  if(!U||!U.is_admin) return;
  api("/api/v1/tenants").then(function(d){
    var all=Array.isArray(d)?d:((d&&d.data)?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Users ('+all.length+') <span id="ln-add-btn"></span></h3><div id="ln-search-wrap" style="margin-bottom:12px"></div><div id="ln-list"></div></div>';
    $("ln-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){ETN()}},"+ Add User"));
    
    // Search bar
    var search=El("input",{style:"width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px",placeholder:"Search users by name or email..."});
    search.oninput=function(){
      var q=search.value.toLowerCase();
      document.querySelectorAll(".ln-row").forEach(function(r){
        var n=(r.getAttribute("data-name")||"").toLowerCase();
        var e=(r.getAttribute("data-email")||"").toLowerCase();
        r.style.display=(n.indexOf(q)>=0||e.indexOf(q)>=0)?"":"none";
      });
    };
    $("ln-search-wrap").appendChild(search);
    
    var list=$("ln-list");
    if(!all.length){list.innerHTML='<p style="color:#64748b;padding:10px">No users.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Name</th><th>Email</th><th>Plan</th><th>Status</th><th></th></tr>';
    all.forEach(function(t){
      var pn=""; if(t.plan&&typeof t.plan==="object") pn=t.plan.plan_name||""; else if(t.plan) pn=String(t.plan);
      var tr=El("tr",{"class":"ln-row"});
      tr.setAttribute("data-name",t.name||"");
      tr.setAttribute("data-email",t.email||"");
      tr.innerHTML='<td><strong>'+X(t.name||"Unnamed")+'</strong></td><td>'+X(t.email||"-")+'</td><td>'+X(pn)+'</td><td><span style="color:'+(t.status==="active"?"#16a34a":"#64748b")+'">'+X(t.status||"active")+'</span></td><td style="white-space:nowrap"></td>';
      var td=tr.querySelectorAll("td")[4];
      td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){ET(t.id,pn)}},"Plan"));
      td.appendChild(document.createTextNode(" "));
      td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){ETN(t.id)}},"Edit"));
      td.appendChild(document.createTextNode(" "));
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){if(confirm("Delete user \""+X(t.name||"")+"\"? This is permanent.")) api("/api/v1/tenants/"+t.id,{method:"DELETE"}).then(function(){S("tenants")});}},"Del"));
      tbl.appendChild(tr);
    });
    list.appendChild(tbl);
  });
}
function ET(tid,pn){
  api("/api/v1/plans").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    O("Assign Plan", function(){
      $("mb").innerHTML='<div class="fld"><label>Current Plan: '+X(pn||"None")+'</label></div>';
      var sel=El("select",{id:"ep-plan"});
      a.forEach(function(p){sel.appendChild(El("option",{value:p.id},X(p.name)+" ($"+(p.price||0)+"/"+(p.billing_interval||"month")+")"));});
      $("mb").appendChild(ff("Select Plan", sel));
      $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
      $("mf").appendChild(El("button",{"class":"btn",onclick:function(){SET(tid)}},"Assign Plan"));
    });
  });
}
function SET(tid){
  var pid=$("ep-plan").value;
  api("/api/v1/tenants/"+tid,{method:"PUT",body:JSON.stringify({plan_id:pid})}).then(function(){C();S("tenants")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}
function ETN(id){
  if(id) api("/api/v1/tenants/"+id).then(function(t){RTN(id,t)});
  else RTN(null,{status:"active"});
}
function RTN(id,t){
  O(id?"Edit User":"Add User", function(){
    var mb=$("mb");
    mb.appendChild(ff("Name", El("input",{id:"fn",value:t.name||""})));
    mb.appendChild(ff("Email", El("input",{id:"fe",value:t.email||""})));
    mb.appendChild(ff("Slug", El("input",{id:"fs",value:t.slug||""})));
    var fst=El("select",{id:"fst"});
    fst.appendChild(El("option",{value:"active",selected:t.status==="active"?"selected":null},"Active"));
    fst.appendChild(El("option",{value:"inactive",selected:t.status==="inactive"?"selected":null},"Inactive"));
    mb.appendChild(ff("Status", fst));
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){STN(id)}},(id?"Save":"Create")));
  });
}
function STN(id){
  var d={name:$("fn").value, email:$("fe").value, slug:$("fs").value, status:$("fst").value};
  if(!d.name||!d.name.trim()){showMsg("Name is required","me");return;}
  var mu=id?"PUT":"POST", u=id?"/api/v1/tenants/"+id:"/api/v1/tenants";
  showMsg("Saving...","mi");
  api(u,{method:mu,body:JSON.stringify(d)}).then(function(){C();S("tenants")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}
function DTN(id,name){if(confirm("Delete tenant "+name+"? Cannot undo.")) api("/api/v1/tenants/"+id,{method:"DELETE"}).then(function(){S("tenants")});}

/* === AFFILIATES === */
function LA(){
  if(!U||!U.is_admin) return;
  api("/api/v1/affiliates").then(function(d){
    var all=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Affiliates ('+all.length+')</h3><div id="la-search-wrap" style="margin-bottom:12px"></div><div id="la-list"></div></div>';
    
    var search=El("input",{style:"width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px",placeholder:"Search affiliates by name, email, or ID..."});
    search.oninput=function(){
      var q=search.value.toLowerCase();
      document.querySelectorAll(".af-row").forEach(function(r){
        var n=(r.getAttribute("data-name")||"").toLowerCase();
        var e=(r.getAttribute("data-email")||"").toLowerCase();
        var i=(r.getAttribute("data-id")||"").toLowerCase();
        r.style.display=(n.indexOf(q)>=0||e.indexOf(q)>=0||i.indexOf(q)>=0)?"":"none";
      });
    };
    $("la-search-wrap").appendChild(search);
    
    if(!all.length){$("la-list").innerHTML='<p style="color:#64748b;padding:20px;text-align:center">No affiliates.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Name</th><th>Email</th><th>ID</th><th>Commission</th><th>Industry</th><th></th></tr>';
    all.forEach(function(x){
      var tr=El("tr",{"class":"af-row"});
      tr.setAttribute("data-name",x.name||"");
      tr.setAttribute("data-email",x.email||"");
      tr.setAttribute("data-id",x.affiliate_id||x.id||"");
      tr.innerHTML='<td><strong>'+X(x.name||"Unnamed")+'</strong></td><td>'+X(x.email||"-")+'</td><td><code>'+X(x.affiliate_id||x.id||"-")+'</code></td><td>'+(x.commission_rate?x.commission_rate+"%":"-")+'</td><td>'+X(x.industry||"-")+'</td><td></td>';
      var td=tr.querySelectorAll("td")[5];
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){if(confirm("Delete affiliate \""+X(x.name||"")+"\"?")) api("/api/v1/affiliates/"+x.id,{method:"DELETE"}).then(function(){S("affiliates")});}},"Del"));
      tbl.appendChild(tr);
    });
    $("la-list").appendChild(tbl);
  });
}

/* === AFFILIATE TIERS (DOM-based) === */
function LTR(){
  if(!U||!U.is_admin) return;
  api("/api/v1/affiliate-tiers").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Affiliate Tiers <span id="ltr-add-btn"></span></h3><p style="color:#64748b;font-size:13px;margin-bottom:12px">Set commission tiers based on lead volume or revenue thresholds.</p><div id="ltr-list"></div></div>';
    $("ltr-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MTR()}},"+ Add Tier"));
    if(!a.length){$("ltr-list").innerHTML='<p style="color:#64748b;padding:10px">No tiers.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Name</th><th>Min Leads</th><th>Min Revenue</th><th>Commission %</th><th>Monthly Cap</th><th></th></tr>';
    a.forEach(function(t){
      var tr=El("tr");
      tr.innerHTML='<td><strong>'+X(t.name||"Tier")+'</strong></td><td>'+(t.min_leads||"-")+'</td><td>$'+(t.min_revenue||"0")+'</td><td>'+(t.commission_percent||"0")+'%</td><td>$'+(t.monthly_cap||"-")+'</td><td></td>';
      var td=tr.querySelectorAll("td")[5];
      td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MTR(t.id)}},"Edit"));
      td.appendChild(document.createTextNode(" "));
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){DTR(t.id)}},"Del"));
      tbl.appendChild(tr);
    });
    $("ltr-list").appendChild(tbl);
  });
}
function MTR(id){
  if(id) api("/api/v1/affiliate-tiers/"+id).then(function(t){RTR(id,t)});
  else RTR(null,{commission_percent:10});
}
function RTR(id,t){
  O(id?"Edit Tier":"Add Tier", function(){
    var mb=$("mb");
    mb.appendChild(ff("Name", El("input",{id:"fn",value:t.name||""})));
    mb.appendChild(ff("Min Leads", El("input",{id:"fml",type:"number",value:t.min_leads||""})));
    mb.appendChild(ff("Min Revenue ($)", El("input",{id:"fmr",type:"number",value:t.min_revenue||""})));
    mb.appendChild(ff("Commission %", El("input",{id:"fcp",type:"number",value:t.commission_percent||10})));
    mb.appendChild(ff("Monthly Cap ($)", El("input",{id:"fmc",type:"number",value:t.monthly_cap||""})));
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){STR(id)}},(id?"Save":"Create")));
  });
}
function STR(id){
  var d={name:$("fn").value, min_leads:parseInt($("fml").value)||null, min_revenue:parseFloat($("fmr").value)||null, commission_percent:parseFloat($("fcp").value)||0, monthly_cap:parseFloat($("fmc").value)||null};
  var m=id?"PUT":"POST", u=id?"/api/v1/affiliate-tiers/"+id:"/api/v1/affiliate-tiers";
  showMsg("Saving...","mi");
  api(u,{method:m,body:JSON.stringify(d)}).then(function(){C();S("tiers")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}
function DTR(id){if(confirm("Delete tier?")) api("/api/v1/affiliate-tiers/"+id,{method:"DELETE"}).then(function(){S("tiers")});}

/* === AFFILIATE PAYOUTS (DOM-based) === */
function LPA(){
  if(!U||!U.is_admin) return;
  api("/api/v1/affiliate-payouts").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Affiliate Payouts</h3><p style="color:#64748b;font-size:13px;margin-bottom:12px">Review and process affiliate commission payouts.</p><div id="lpa-list"></div></div>';
    if(!a.length){$("lpa-list").innerHTML='<p style="color:#64748b;padding:10px">No payouts.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Affiliate</th><th>Amount</th><th>Period</th><th>Status</th><th></th></tr>';
    a.forEach(function(p){
      var tr=El("tr");
      tr.innerHTML='<td>'+X(p.affiliate_name||p.affiliate_id||"-")+'</td><td>$'+(p.amount||"0")+'</td><td>'+X(p.period||"-")+'</td><td><span class="badge '+(p.status==="paid"?"bg":"bb")+'">'+X(p.status||"pending")+'</span></td><td></td>';
      var td=tr.querySelectorAll("td")[4];
      if(p.status!=="paid") td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MPP(p.id)}},"Mark Paid"));
      tbl.appendChild(tr);
    });
    $("lpa-list").appendChild(tbl);
  });
}
function MPP(id){if(confirm("Mark payout as paid?")) api("/api/v1/affiliate-payouts/"+id+"/pay",{method:"POST"}).then(function(){S("payouts")});}

/* === PLANS === */
function LPLL(){
  if(!U||!U.is_admin) return;
  api("/api/v1/plans").then(function(d){
    var plans=Array.isArray(d)?d:((d&&d.data)?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Plans ('+plans.length+')</h3><div id="lpll-list"></div></div>';
    if(!plans.length){$("lpll-list").innerHTML='<p style="color:#64748b;padding:20px">No plans.</p>';return;}

    var root=$("lpll-list");
    plans.forEach(function(p){
      var f=p.features||{};
      var card=El("div",{style:"background:#fff;border:1px solid #e2e8f0;border-radius:10px;padding:18px;margin-bottom:12px"});
      
      // Header row
      var hdr=El("div",{style:"display:flex;justify-content:space-between;align-items:center;margin-bottom:12px"});
      hdr.innerHTML='<div><strong style="font-size:16px;color:#1e1b4b">'+X(p.name)+'</strong><span style="margin-left:10px;font-size:12px;color:#94a3b8">/'+X(p.slug)+'</span></div><div style="display:flex;gap:8px;align-items:center"><span style="font-size:13px;font-weight:700;color:#6366f1">$'+X(String(p.price||0))+'/mo</span>'+'</div>';
      card.appendChild(hdr);

      // Editable top-level fields
      var topFields=El("div",{style:"display:grid;grid-template-columns:repeat(4,1fr);gap:8px;margin-bottom:14px;padding:10px;background:#f8fafc;border-radius:8px"});
      topFields.appendChild(PFF("Name",p.id,"name",p.name));
      topFields.appendChild(PFF("Slug",p.id,"slug",p.slug));
      topFields.appendChild(PFF("Price ($)",p.id,"price",String(p.price||0),"number"));
      topFields.appendChild(PFF("Max Leads",p.id,"max_leads",String(p.max_leads||"0"),"number"));
      topFields.appendChild(PFF("Max Tags",p.id,"max_tags",String(p.max_tags||"0"),"number"));
      topFields.appendChild(PFF("Max Users",p.id,"max_users",String(f.max_users||"0"),"number"));
      topFields.appendChild(PFF("Max Cards",p.id,"max_kinetic_cards",String(f.max_kinetic_cards||"0"),"number"));
      topFields.appendChild(PFF("CTA Buttons",p.id,"kinetic_cta_buttons_max",String(f.kinetic_cta_buttons_max||"0"),"number"));
      card.appendChild(topFields);

      // Booleans
      var bools=El("div",{style:"margin-bottom:4px"});
      bools.appendChild(El("p",{style:"font-size:11px;font-weight:600;color:#64748b;margin-bottom:8px"},"Flags"));
      var bg=El("div",{style:"display:grid;grid-template-columns:repeat(4,1fr);gap:4px;margin-bottom:10px"});
      [{k:"has_dual_routing",l:"Dual Routing"},{k:"has_multi_tenant",l:"Multi-Tenant"},{k:"has_white_label",l:"White Label"}].forEach(function(fb){
        bg.appendChild(PFC(p.id,fb.k,fb.l,!!p[fb.k]));
      });
      bools.appendChild(bg);
      card.appendChild(bools);

      // Feature toggles
      var ft=El("div",{style:"margin-bottom:10px"});
      ft.appendChild(El("p",{style:"font-size:11px;font-weight:600;color:#64748b;margin-bottom:8px"},"Features"));
      var fg=El("div",{style:"display:grid;grid-template-columns:repeat(4,1fr);gap:4px"});
      var featList=[
        {k:"kinetic_branding",l:"Card Branding"},
        {k:"kinetic_custom_colors",l:"Custom Colors"},
        {k:"kinetic_custom_domain",l:"Custom Domain"},
        {k:"kinetic_video",l:"Video Embed"},
        {k:"kinetic_minipage",l:"Mini Pages"},
        {k:"kinetic_minifunnel",l:"Mini Funnels"},
        {k:"kinetic_source_tracking",l:"Source Tracking"},
        {k:"kinetic_analytics",l:"Analytics"},
        {k:"kinetic_theme_templates",l:"Theme Templates"},
        {k:"analytics",l:"Global Analytics"},
        {k:"api_access",l:"API Access"},
        {k:"import_export",l:"Import/Export"},
        {k:"dedicated_support",l:"Dedicated Support"},
        {k:"white_label",l:"White Label"},
        {k:"kinetic_social_links_max",l:"Social Links"},
        {k:"kinetic_plan",l:"Plan Selector"},
      ];
      featList.forEach(function(fb){
        fg.appendChild(PFC(p.id,(fb.k==="kinetic_social_links_max"?"features":null),fb.k,fb.l,(fb.k==="kinetic_social_links_max"?f[fb.k]:!!f[fb.k])));
      });
      ft.appendChild(fg);
      card.appendChild(ft);

      // CTA text
      var ctDiv=El("div",{style:"padding:8px;background:#f8fafc;border-radius:6px;margin-bottom:8px"});
      ctDiv.appendChild(PFF("CTA Text",p.id,"kinetic_cta_text",f.kinetic_cta_text||"Claim Your {{type}}"));
      card.appendChild(ctDiv);

      // Save button
      var act=El("div",{style:"display:flex;gap:8px;align-items:center"});
      act.appendChild(El("button",{"class":"btn",onclick:function(){SP(p.id)}},"Save "+X(p.name)));
      var sg=El("span",{style:"font-size:11px;color:#94a3b8"});
      act.appendChild(sg);
      card.appendChild(act);
      card._status=sg;
      card._pid=p.id;
      
      root.appendChild(card);
    });
  });
}

// Plan field — inline editable input
function PFF(label,pid,key,val,type){
  var d=El("div",{style:"display:flex;flex-direction:column;gap:2px"});
  d.appendChild(El("label",{style:"font-size:10px;color:#64748b;font-weight:600"},label));
  var inp=El("input",{type:type||"text",value:val||"",style:"padding:4px 6px;border:1px solid #d1d5db;border-radius:4px;font-size:11px",placeholder:label});
  inp.setAttribute("data-plan",pid);
  inp.setAttribute("data-key",key);
  d.appendChild(inp);
  return d;
}
// Plan feature checkbox
function PFC(pid,prefix,key,label,checked){
  var d=El("label",{style:"display:flex;align-items:center;gap:4px;font-size:11px;color:#334155;cursor:pointer;padding:2px 4px;border-radius:4px"});
  var cb=El("input",{type:"checkbox",checked:checked?"checked":null,style:"width:14px;height:14px"});
  cb.setAttribute("data-plan",pid);
  cb.setAttribute("data-key",key);
  if(prefix) cb.setAttribute("data-prefix",prefix);
  d.appendChild(cb);
  d.appendChild(document.createTextNode(label));
  return d;
}

// Save plan
function SP(pid){
  var d={};
  var feats={};
  document.querySelectorAll('[data-plan="'+pid+'"]').forEach(function(el){
    var k=el.getAttribute("data-key");
    var v; var isCB=el.type==="checkbox";
    if(isCB){
      v=el.checked;
      var pf=el.getAttribute("data-prefix");
      if(pf==="features"){feats[k]=v;return;}
    }else{
      var nt=el.type;
      if(nt==="number"){v=parseFloat(el.value)||0;if(v===-1)v=-1;}
      else v=el.value;
    }
    // Map features.* fields
    if(k.startsWith("kinetic_")||k==="max_kinetic_cards"||k==="max_users"||k==="analytics"||k==="api_access"||k==="import_export"||k==="dedicated_support"||k==="white_label"){
      if(!isCB&&k!=="kinetic_cta_text") feats[k]=v;
      else if(k==="kinetic_cta_text") feats[k]=v;
    }else{
      d[k]=v;
    }
  });
  d.features=feats;
  
  showMsg("Saving...","mi");
  api("/api/v1/plans/"+pid,{method:"PUT",body:JSON.stringify(d)}).then(function(){
    showMsg("Plan saved!","mk");
    setTimeout(function(){$("mm").className="msg"},2000);
  }).catch(function(e){showMsg(e.message||"Save failed","me")});
}


/* === WEBHOOKS === */
function LW(){
  if(!U||!U.is_admin) return;
  api("/api/v1/webhooks").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>Webhooks ('+a.length+')</h3><div id="lw-list"></div></div>';
    if(!a.length){$("lw-list").innerHTML='<p style="color:#64748b;padding:20px;text-align:center">No webhooks.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Event</th><th>URL</th><th>Created</th><th></th></tr>';
    a.forEach(function(w){
      var tr=El("tr");
      tr.innerHTML='<td>'+X(w.event||"-")+'</td><td style="max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">'+X(w.url||"-")+'</td><td>'+(w.created_at?String(w.created_at).slice(0,10):"-")+'</td><td></td>';
      var td=tr.querySelectorAll("td")[3];
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){DW(w.id)}},"Del"));
      tbl.appendChild(tr);
    });
    $("lw-list").appendChild(tbl);
  });
}
function DW(id){if(confirm("Delete?")) api("/api/v1/webhooks/"+id,{method:"DELETE"}).then(function(){S("webhooks")});}

/* === API KEYS === */
function LK(){
  if(!U||!U.is_admin) return;
  api("/api/v1/api-keys").then(function(d){
    var a=Array.isArray(d)?d:(d&&d.data?d.data:[]);
    $("ct").innerHTML='<div class="cc"><h3>API Keys ('+a.length+') <span id="lk-add-btn"></span></h3><div id="lk-list"></div></div>';
    $("lk-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:GKA},"+ Generate"));
    if(!a.length){$("lk-list").innerHTML='<p style="color:#64748b;padding:20px;text-align:center">None.</p>';return;}
    var tbl=El("table");
    tbl.innerHTML='<tr><th>Name</th><th>Prefix</th><th>Created</th><th></th></tr>';
    a.forEach(function(k){
      var tr=El("tr");
      tr.innerHTML='<td>'+X(k.name||"-")+'</td><td><code>'+X(k.prefix||k.key||"-")+'</code></td><td>'+(k.created_at?String(k.created_at).slice(0,10):"-")+'</td><td></td>';
      var td=tr.querySelectorAll("td")[3];
      td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){DK(k.id)}},"Revoke"));
      tbl.appendChild(tr);
    });
    $("lk-list").appendChild(tbl);
  });
}
function GKA(){api("/api/v1/api-keys",{method:"POST",body:JSON.stringify({name:"Admin Key"})}).then(function(r){if(r&&r.key) alert("Key: "+r.key); S("keys");});}
function DK(id){if(confirm("Revoke?")) api("/api/v1/api-keys/"+id,{method:"DELETE"}).then(function(){S("keys")});}

/* === SYSTEM TAGS (Admin) === */
function LST(){
  if(!U||!U.is_admin) return;
  Promise.all([api("/api/v1/tags"),api("/api/v1/tag-groups")]).then(function(r){
    var tags=Array.isArray(r[0])?r[0]:((r[0]&&r[0].data)?r[0].data:[]);
    var groups=Array.isArray(r[1])?r[1]:((r[1]&&r[1].data)?r[1].data:[]);
    var gName={};
    groups.forEach(function(g){gName[g.id]=g.name;});
    
    var sys=[], cust=[];
    tags.forEach(function(t){if(t.is_system) sys.push(t);else cust.push(t);});
    
    $("ct").innerHTML='<div class="cc"><h3>System Tags ('+sys.length+') <span id="lst-add-btn"></span><span id="lst-bulk-span"></span></h3><p style="color:#64748b;font-size:12px;margin-bottom:12px">System tags are shared across all tenants. Custom tags belong to individual users.<br><span style="color:#f59e0b">Note: Editing system tag colors/names requires a backend update — PUT currently blocked.</span></p><div id="lst-search-wrap" style="margin-bottom:12px"></div><div id="lst-content"></div></div>';
    $("lst-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFT(null,true)}},"+ Add System Tag"));
    
    var search=El("input",{style:"width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px",placeholder:"Search tags by name..."});
    search.oninput=function(){
      var q=search.value.toLowerCase();
      document.querySelectorAll(".st-row").forEach(function(r){
        r.style.display=(r.getAttribute("data-name")||"").toLowerCase().indexOf(q)>=0?"":"none";
      });
    };
    $("lst-search-wrap").appendChild(search);$("lst-bulk-span").appendChild(El("button",{"class":"btn btn-sm btn-r",style:"margin-left:8px",onclick:LSTBD},"Bulk Delete"));
    
    var ct=$("lst-content");
    
    if(!sys.length){ct.appendChild(El("p",{style:"color:#94a3b8;padding:10px"},"No system tags."));}
    else{
      var tbl=El("table");
      tbl.innerHTML='<tr><th>Tag</th><th>Color</th><th>Group</th><th>Created</th><th></th></tr>';
      sys.forEach(function(t){
        var gn=gName[t.group_id]||(t.group_id?t.group_id.slice(0,8)+"...":"-");
        var tr=El("tr",{"class":"st-row"});
        tr.setAttribute("data-name",t.name||"");
        tr.innerHTML='<td><span class="tag" style="background:'+X(t.color||"#6366f1")+'20;color:'+X(t.color||"#6366f1")+';padding:3px 10px;border-radius:4px;font-size:12px;font-weight:600">'+X(t.name)+'</span></td><td><span style="display:inline-block;width:12px;height:12px;background:'+X(t.color||"#6366f1")+';border-radius:3px;vertical-align:middle;margin-right:4px"></span> '+X(t.color||"#6366f1")+'</td><td>'+X(gn)+'</td><td>'+(t.created_at?String(t.created_at).slice(0,10):"-")+'</td><td></td>';
        var td=tr.querySelectorAll("td")[4];
        td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFT(t.id,true)}},"Edit"));
        td.appendChild(document.createTextNode(" "));
        td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){if(confirm("Delete system tag \""+X(t.name)+"\"? This removes it for ALL tenants.")) api("/api/v1/tags/"+t.id,{method:"DELETE"}).then(function(){S("stags")});}},"Del"));
        tbl.appendChild(tr);
      });
      ct.appendChild(tbl);
    }
  });
}

/* === SYSTEM TAG GROUPS (Admin) === */
function LSG(){
  if(!U||!U.is_admin) return;
  Promise.all([api("/api/v1/tag-groups"),api("/api/v1/tags")]).then(function(r){
    var groups=Array.isArray(r[0])?r[0]:(r[0]&&r[0].data?r[0].data:[]);
    var tags=Array.isArray(r[1])?r[1]:(r[1]&&r[1].data?r[1].data:[]);
    
    // Backend doesn't return is_system for tag groups, so admin sees ALL groups
    // Count tags per group
    var tgCount={};
    tags.forEach(function(t){var g=t.group_id;if(g)tgCount[g]=(tgCount[g]||0)+1;});
    
    $("ct").innerHTML='<div class="cc"><h3>System Tag Groups ('+groups.length+') <span id="lsg-add-btn"></span></h3><p style="color:#64748b;font-size:12px;margin-bottom:12px">Tag groups organize tags into categories. Admin manages all groups here.</p><div id="lsg-search-wrap" style="margin-bottom:12px"></div><div id="lsg-content"></div></div>';
    $("lsg-add-btn").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFG(null,true)}},"+ Add Group"));
    
    // Search
    var search=El("input",{style:"width:100%;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px",placeholder:"Search groups by name..."});
    search.oninput=function(){
      var q=search.value.toLowerCase();
      document.querySelectorAll(".sg-row").forEach(function(r){
        r.style.display=(r.getAttribute("data-name")||"").toLowerCase().indexOf(q)>=0?"":"none";
      });
    };
    $("lsg-search-wrap").appendChild(search);
    
    var ct=$("lsg-content");
    
    if(!groups.length){ct.appendChild(El("p",{style:"color:#94a3b8;padding:10px"},"No tag groups. Create one to organize tags."));}
    else{
      var tbl=El("table");
      tbl.innerHTML='<tr><th>Group</th><th>Tags</th><th>Sort</th><th>Created</th><th></th></tr>';
      groups.forEach(function(g){
        var cnt=tgCount[g.id]||0;
        var tr=El("tr",{"class":"sg-row"});
        tr.setAttribute("data-name",g.name||"");
        tr.innerHTML='<td><strong>'+X(g.name)+'</strong></td><td>'+cnt+' tags</td><td>'+(g.sort_order||0)+'</td><td>'+(g.created_at?String(g.created_at).slice(0,10):"-")+'</td><td></td>';
        var td=tr.querySelectorAll("td")[4];
        td.appendChild(El("button",{"class":"btn btn-sm",onclick:function(){MFG(g.id,false)}},"Edit"));
        td.appendChild(document.createTextNode(" "));
        td.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){if(confirm("Delete group \""+X(g.name)+"\"? Tags will be ungrouped.")) api("/api/v1/tag-groups/"+g.id,{method:"DELETE"}).then(function(){S("sgroups")});}},"Del"));
        tbl.appendChild(tr);
      });
      ct.appendChild(tbl);
    }
  });
}

/* === Updated tag edit — handle system flag === */
function MFT(id,isSystem){
  api("/api/v1/tag-groups").then(function(gd){
    var groups=Array.isArray(gd)?gd:((gd&&gd.data)?gd.data:[]);
    if(id) api("/api/v1/tags/"+id).then(function(t){RFT(id,t,groups,isSystem)});
    else RFT(null,{color:"#6366f1",is_system:!!isSystem},groups,isSystem);
  });
}
function RFT(id, t, groups, isSystem){
  O(id?"Edit "+((t.is_system||isSystem)?"System Tag":"Tag"):"Create "+((t.is_system||isSystem)?"System Tag":"Tag"), function(){
    var mb=$("mb"); mb.innerHTML=""; $("mf").innerHTML="";
    mb.appendChild(ff("Name", El("input",{id:"fn",value:t.name||""})));
    var fg=El("select",{id:"fg"});
    fg.appendChild(El("option",{value:""},"None"));
    groups.forEach(function(g){
      fg.appendChild(El("option",{value:g.id,selected:t.group_id===g.id?"selected":null},X(g.name)));
    });
    mb.appendChild(ff("Tag Group", fg));
    mb.appendChild(ff("Color", El("input",{id:"fc",type:"color",value:t.color||"#6366f1"})));
    if(t.is_system||isSystem){
      var info=El("div",{style:"background:#fef3c7;padding:8px 12px;border-radius:6px;font-size:12px;color:#92400e;margin-top:8px"});
      info.textContent="⚠️ System tags are shared across all tenants. Editing may be restricted.";
      mb.appendChild(info);
    }
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){ST(id,isSystem?"stags":"tags",isSystem)}},(id?"Save":"Create")));
  });
}
function ST(id,returnTab,isSystem){
  var d={name:$("fn").value, group_id:$("fg").value||null, color:$("fc").value};
  if(isSystem) d.is_system=true;
  if(!d.name||!d.name.trim()){showMsg("Name is required","me");return;}
  var mu=id?"PUT":"POST", u=id?"/api/v1/tags/"+id:"/api/v1/tags";
  showMsg("Saving...","mi");
  api(u,{method:mu,body:JSON.stringify(d)}).then(function(){C();S(returnTab||"tags")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}

/* === LEAD STAGES (Admin) === */
function LSTBD(){
  var cbs=document.querySelectorAll(".sys-tag-cb:checked");
  if(!cbs.length){showMsg("Select tags to delete","me");return;}
  if(!confirm("Delete "+cbs.length+" system tag(s)? This affects all tenants.")) return;
  var remaining=cbs.length;
  var errors=[];
  cbs.forEach(function(cb){
    api("/api/v1/tags/"+cb.value,{method:"DELETE"}).then(function(){
      remaining--;
      if(remaining===0){
        showMsg(errors.length?errors.length+" failed":"Deleted successfully","ms");
        S("stags");
      }
    }).catch(function(e){remaining--;errors.push(cb.value);});
  });
}


function LLS(){
  if(!U||!U.is_admin) return;
  api("/api/v1/settings").then(function(d){
    var stages=["New","Contacted","Qualified","Proposal","Negotiation","Closed Won","Closed Lost"];
    d.forEach(function(s){if(s.key==="lead_stages"&&Array.isArray(s.value)) stages=s.value;});
    
    $("ct").innerHTML='<div class="cc"><h3>Lead Pipeline Stages</h3><p style="color:#64748b;font-size:12px;margin-bottom:8px">Click a stage to edit it. Add/remove stages below.</p><div id="lls-list"></div><div id="lls-add" style="margin-top:12px;display:flex;gap:8px"></div></div>';
    
    var list=$("lls-list");
    LLSR(stages);
    
    var addInp=El("input",{placeholder:"New stage name",style:"flex:1;padding:8px 10px;border:1px solid #d1d5db;border-radius:6px;font-size:13px"});
    $("lls-add").appendChild(addInp);
    $("lls-add").appendChild(El("button",{"class":"btn btn-sm",onclick:function(){
      var n=addInp.value.trim(); if(!n) return;
      var curs=LLSR(null);
      curs.push(n);
      api("/api/v1/settings",{method:"PUT",body:JSON.stringify({key:"lead_stages",value:curs})}).then(function(){S("lead-stages")}).catch(function(e){showMsg(e.message,"me")});
    }},"Add Stage"));
    $("lls-add").appendChild(El("button",{"class":"btn btn-sm btn-o",onclick:function(){
      var curs=LLSR(null);
      showMsg("Click Save to save reordering (drag handles coming)","mi");
    }},"Save Order"));
  });
}
function LLSR(stages){
  if(stages){window._lls_stages=stages;}
  var stages=window._lls_stages||[];
  var list=$("lls-list");
  if(!list) return stages;
  list.innerHTML="";
  stages.forEach(function(s,i){
    var row=El("div",{style:"display:flex;align-items:center;gap:8px;padding:8px 10px;margin-bottom:4px;background:#f8fafc;border:1px solid #e2e8f0;border-radius:6px"});
    row.appendChild(El("span",{style:"font-size:16px;color:#94a3b8;cursor:grab"},"≡"));
    row.appendChild(El("span",{style:"width:22px;text-align:center;font-weight:700;color:#6366f1;font-size:12px"},String(i+1)));
    var inp=El("input",{value:s,style:"flex:1;padding:4px 8px;border:1px solid transparent;border-radius:4px;font-size:13px;background:transparent"});
    inp.onchange=function(){
      window._lls_stages[i]=inp.value;
      var s2=LLSR(null);
      api("/api/v1/settings",{method:"PUT",body:JSON.stringify({key:"lead_stages",value:s2})}).then(function(){showMsg("Saved","mk");setTimeout(function(){$("mm").className="msg"},1500)});
    };
    row.appendChild(inp);
    row.appendChild(El("button",{"class":"btn btn-sm btn-r",onclick:function(){
      window._lls_stages.splice(i,1);
      var ns=LLSR(null);
      api("/api/v1/settings",{method:"PUT",body:JSON.stringify({key:"lead_stages",value:ns})}).then(function(){S("lead-stages")}).catch(function(e){showMsg(e.message,"me")});
    }},"×"));
    list.appendChild(row);
  });
  return stages.slice();
}

/* === SETTINGS (Admin) === */
function LM(){
  if(!U||!U.is_admin) return;
  api("/api/v1/settings").then(function(d){
    $("ct").innerHTML='<div class="cc"><h3>Global Settings</h3><p style="color:#64748b;font-size:12px;margin-bottom:12px">System-wide configuration.</p><div id="lm-list"></div></div>';
    
    var list=$("lm-list");
    var sd=d.find(function(s){return s.key==="subdomain"})||{value:"ss"};
    var cd=d.find(function(s){return s.key==="custom_domain"})||{value:""};
    var ls=d.find(function(s){return s.key==="lead_stages"});
    
    // Subdomain
    var sdDiv=El("div",{style:"border:1px solid #e2e8f0;border-radius:8px;padding:14px;margin-bottom:10px"});
    sdDiv.innerHTML='<div style="display:flex;justify-content:space-between;align-items:center"><div><strong style="font-size:13px">Card Subdomain</strong><br><code style="font-size:10px;color:#94a3b8">subdomain</code><br><span style="font-size:12px;color:#6366f1">'+X(sd.value)+'.kntcrd.com</span></div><div style="display:flex;gap:6px;align-items:center"><input id="lmsd" value="'+X(sd.value)+'" style="width:120px;padding:6px 8px;border:1px solid #d1d5db;border-radius:6px;font-size:12px"><button class="btn btn-sm" onclick="var v=document.getElementById(\'lmsd\').value.trim();if(!v)return;api(\'/api/v1/settings\',{method:\'PUT\',body:JSON.stringify({key:\'subdomain\',value:v})}).then(function(){showMsg(\'Saved\',\'mk\');setTimeout(function(){$(\'mm\').className=\'msg\'},1500)})">Save</button></div></div>';
    list.appendChild(sdDiv);
    
    // Custom domain
    var cdDiv=El("div",{style:"border:1px solid #e2e8f0;border-radius:8px;padding:14px;margin-bottom:10px"});
    cdDiv.innerHTML='<div style="display:flex;justify-content:space-between;align-items:center"><div><strong style="font-size:13px">Default Custom Domain</strong><br><code style="font-size:10px;color:#94a3b8">custom_domain</code></div><div style="display:flex;gap:6px;align-items:center"><input id="lmcd" value="'+X(cd.value||"")+'" placeholder="optional" style="width:180px;padding:6px 8px;border:1px solid #d1d5db;border-radius:6px;font-size:12px"><button class="btn btn-sm" onclick="var v=document.getElementById(\'lmcd\').value.trim();api(\'/api/v1/settings\',{method:\'PUT\',body:JSON.stringify({key:\'custom_domain\',value:v})}).then(function(){showMsg(\'Saved\',\'mk\');setTimeout(function(){$(\'mm\').className=\'msg\'},1500)})">Save</button></div></div>';
    list.appendChild(cdDiv);
    
    // Lead stages
    var lsVal=ls?ls.value:["New","Contacted","Qualified"];
    var lsDiv=El("div",{style:"border:1px solid #e2e8f0;border-radius:8px;padding:14px;margin-bottom:10px"});
    lsDiv.innerHTML='<div style="display:flex;justify-content:space-between;align-items:center"><div><strong style="font-size:13px">Lead Pipeline Stages</strong><br><span style="font-size:11px;color:#94a3b8">Click to edit</span></div><button class="btn btn-sm" onclick="S(\'lead-stages\')">Edit</button></div>';
    var stagesDisp=Array.isArray(lsVal)?lsVal.join(" → "):String(lsVal);
    lsDiv.appendChild(El("p",{style:"color:#6366f1;font-size:12px;margin-top:8px;font-weight:500"},stagesDisp));
    list.appendChild(lsDiv);
  });
}
function showMsg(msg,cls){
  $("mm").className="msg "+cls; $("mm").textContent=msg;
}

/* ═══ ADMIN SEO SETTINGS ═══ */
function LSEO(){
  if(!U||!U.is_admin) return;
  api("/api/v1/seo/settings").then(function(d){
    var seo=typeof d==="object"?d:{};
    $("ct").innerHTML='<div class="cc"><h3>🎛️ SEO Settings</h3><p style="color:#64748b;font-size:12px;margin-bottom:12px">Control how search engines see your site. All fields optional — leave blank to use defaults.</p><div id="seo-fields"></div><div style="margin-top:16px;display:flex;gap:8px"><button class="btn" onclick="SEOSave()">Save All</button><button class="btn btn-o" onclick="S(\'seo\')">Reload</button></div><p id="seo-msg" class="msg" style="margin-top:8px"></p><hr style="margin:20px 0;border-color:#e2e8f0"><h3 style="font-size:14px;margin-bottom:8px">📡 Public Endpoints</h3><p style="font-size:12px;color:#64748b"><a href="/api/v1/seo/sitemap.xml" target="_blank" style="color:#6366f1">/api/v1/seo/sitemap.xml</a> — Dynamic sitemap (all cards + funnels)</p><p style="font-size:12px;color:#64748b"><a href="/robots.txt" target="_blank" style="color:#6366f1">/robots.txt</a> — Dynamic crawl rules</p></div>';

    var fields=[
      {k:"site_name", label:"Site Name", ph:"FunnelSwift", desc:"Shown in browser tabs & Open Graph"},
      {k:"description", label:"Meta Description", ph:"Lead generation & affiliate marketing platform", desc:"155-160 chars. Appears in search results."},
      {k:"keywords", label:"Meta Keywords", ph:"lead generation, affiliate marketing, CRM", desc:"Comma-separated."},
      {k:"og_image", label:"Open Graph Image URL", ph:"https://funnelswift.net/assets/og-banner.png", desc:"Image shown when shared on Facebook/LinkedIn/Slack. 1200×630px."},
      {k:"twitter_handle", label:"Twitter/X Handle", ph:"@funnelswift", desc:"Twitter card attribution (include @)."},
      {k:"google_analytics", label:"Google Analytics ID", ph:"G-XXXXXXXXXX or UA-XXXXXXXXX-X", desc:"GA4 or Universal Analytics tracking ID."},
      {k:"facebook_pixel", label:"Facebook Pixel ID", ph:"1234567890", desc:"Facebook Pixel ID for conversion tracking."},
      {k:"site_verification", label:"Google Site Verification", ph:"verification-token-from-google", desc:"Content value from Google Search Console verification."}
    ];

    var html='<div style="display:grid;grid-template-columns:1fr 1fr;gap:12px" id="seo-grid">';
    fields.forEach(function(f){
      var v=seo[f.k]||"";
      html+='<div style="border:1px solid #e2e8f0;border-radius:8px;padding:12px"><label style="font-size:12px;font-weight:600;color:#475569;display:block;margin-bottom:4px">'+f.label+'</label><p style="font-size:10px;color:#94a3b8;margin-bottom:6px">'+f.desc+'</p><input id="seo-'+f.k+'" value="'+X(v)+'" placeholder="'+f.ph+'" style="width:100%;padding:6px 8px;border:1px solid #d1d5db;border-radius:6px;font-size:12px;font-family:inherit"></div>';
    });
    html+='</div>';

    // Schema type selector
    html+='<div style="margin-top:14px;border:1px solid #e2e8f0;border-radius:8px;padding:14px"><label style="font-size:12px;font-weight:600;color:#475569">Schema Markup Type</label><p style="font-size:10px;color:#94a3b8;margin-bottom:6px">Google structured data type for your company.</p>';
    var schemaTypes=["Organization","LocalBusiness","SoftwareApplication"];
    var currentSchema=seo.schema_type||"SoftwareApplication";
    html+='<select id="seo-schema_type" style="width:240px;padding:6px 8px;border:1px solid #d1d5db;border-radius:6px;font-size:12px">';
    schemaTypes.forEach(function(t){ html+='<option value="'+t+'"'+(currentSchema===t?' selected':'')+'>'+t+'</option>'; });
    html+='</select>';

    // Company info fields for schema
    html+='<div style="display:grid;grid-template-columns:1fr 1fr;gap:8px;margin-top:8px">';
    [
      {k:"company_name", label:"Company Name", ph:"FunnelSwift"},
      {k:"company_logo", label:"Logo URL", ph:"https://funnelswift.net/logo.png"},
      {k:"company_address", label:"Address", ph:"123 Main St, City, State"},
      {k:"company_phone", label:"Phone", ph:"+1-555-555-5555"},
      {k:"company_email", label:"Email", ph:"hello@funnelswift.net"}
    ].forEach(function(cf){
      var cv=seo[cf.k]||"";
      html+='<div><label style="font-size:11px;color:#64748b">'+cf.label+'</label><input id="seo-'+cf.k+'" value="'+X(cv)+'" placeholder="'+cf.ph+'" style="width:100%;padding:6px 8px;border:1px solid #d1d5db;border-radius:6px;font-size:12px;font-family:inherit"></div>';
    });
    html+='</div></div>';

    $("seo-fields").innerHTML=html;
  }).catch(function(e){ $("ct").innerHTML='<div class="msg me">Failed to load SEO settings: '+e.message+'</div>'; });
}
function SEOSave(){
  var keys=["site_name","description","keywords","og_image","twitter_handle","google_analytics","facebook_pixel","site_verification","schema_type","company_name","company_logo","company_address","company_phone","company_email"];
  var payload={};
  keys.forEach(function(k){
    var el=document.getElementById("seo-"+k);
    if(el&&el.value.trim()) payload[k]=el.value.trim();
  });
  // Build schema_type JSON
  var schemaType=document.getElementById("seo-schema_type");
  if(schemaType&&schemaType.value){
    var schema={};
    schema["@type"]=schemaType.value;
    var companyName=payload["company_name"]||"FunnelSwift";
    schema.name=companyName;
    if(payload.company_logo) schema.logo=payload.company_logo;
    if(schemaType.value==="LocalBusiness"||schemaType.value==="Organization"){
      if(payload.company_address) schema.address={streetAddress:payload.company_address};
      if(payload.company_phone) schema.telephone=payload.company_phone;
      if(payload.company_email) schema.email=payload.company_email;
    }
    payload.schema_type=JSON.stringify(schema);
    delete payload.company_name; delete payload.company_logo; delete payload.company_address;
    delete payload.company_phone; delete payload.company_email;
  }

  var msg=$("seo-msg");
  msg.className="msg mi"; msg.textContent="Saving...";
  api("/api/v1/seo/settings",{method:"PUT",body:JSON.stringify(payload)}).then(function(){
    msg.className="msg ms"; msg.textContent="✅ SEO settings saved!";
    setTimeout(function(){msg.className="msg"},3000);
  }).catch(function(e){
    msg.className="msg me"; msg.textContent="❌ "+e.message;
  });
}

/* === Updated group edit — handle system flag === */
function MFG(id,isSystem){
  if(id) api("/api/v1/tag-groups/"+id).then(function(g){RFG(id,g,isSystem)});
  else RFG(null,{is_collapsible:true,sort_order:0,is_system:!!isSystem},isSystem);
}
function RFG(id,g,isSystem){
  O(id?"Edit "+((g.is_system||isSystem)?"System Group":"Tag Group"):"Create "+((g.is_system||isSystem)?"System Group":"Tag Group"), function(){
    var mb=$("mb"); mb.innerHTML=""; $("mf").innerHTML="";
    mb.appendChild(ff("Name", El("input",{id:"fn",value:g.name||""})));
    mb.appendChild(ff("Collapsible", El("input",{id:"fc",type:"checkbox",checked:g.is_collapsible?"checked":null})));
    mb.appendChild(ff("Sort Order", El("input",{id:"fo",type:"number",value:g.sort_order||0})));
    if(g.is_system||isSystem){
      var info=El("div",{style:"background:#fef3c7;padding:8px 12px;border-radius:6px;font-size:12px;color:#92400e;margin-top:8px"});
      info.textContent="⚠️ System tag groups organize system tags across all tenants.";
      mb.appendChild(info);
    }
    $("mf").appendChild(El("button",{"class":"btn btn-o",onclick:C},"Cancel"));
    $("mf").appendChild(El("button",{"class":"btn",onclick:function(){SG(id,isSystem?"sgroups":"tg",isSystem)}},(id?"Save":"Create")));
  });
}
function SG(id,returnTab,isSystem){
  var d={name:$("fn").value, is_collapsible:$("fc").checked, sort_order:parseInt($("fo").value)||0};
  if(isSystem) d.is_system=true;
  var m=id?"PUT":"POST", u=id?"/api/v1/tag-groups/"+id:"/api/v1/tag-groups";
  showMsg("Saving...","mi");
  api(u,{method:m,body:JSON.stringify(d)}).then(function(){C();S(returnTab||"tg")}).catch(function(e){showMsg(e.message||"Save failed","me")});
}

