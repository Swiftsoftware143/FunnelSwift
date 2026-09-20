


// ── Funnel Builder ──

var __funnelCache = null;

function renderFunnels(el){
  el.innerHTML = '<div class="page-header"><div><h2>&#x1f517; Funnel Builder</h2><p>Chain kinetic cards into a step-by-step funnel. Each card CTA advances to the next step.</p></div><div><button class="btn btn-primary" onclick="showFunnelForm()">+ New Funnel</button></div></div><div class="card" style="padding:24px;text-align:center"><p style="color:#64748b;margin-bottom:12px">Loading funnels...</p></div>';
  loadFunnels(el);
}

async function loadFunnels(el){
  try {
    var resp = await api('GET','/funnels');
    var funnels = resp.funnels || [];
    __funnelCache = funnels;
    if(!funnels.length){
      el.innerHTML = '<div class="page-header"><div><h2>&#x1f517; Funnel Builder</h2><p>Chain kinetic cards into step-by-step funnels.</p></div><div><button class="btn btn-primary" onclick="showFunnelForm()">+ New Funnel</button></div></div><div class="card" style="padding:40px;text-align:center"><p style="color:#94a3b8;font-size:15px">No funnels yet</p><p style="color:#64748b;font-size:13px;margin-top:4px">Create your first funnel to start capturing leads through a step-by-step flow.</p></div>';
      return;
    }
    var h = '<div class="page-header"><div><h2>&#x1f517; Funnel Builder</h2><p>Chain kinetic cards into step-by-step funnels.</p></div><div><button class="btn btn-primary" onclick="showFunnelForm()">+ New Funnel</button></div></div>';
    h += '<div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:12px">';
    funnels.forEach(function(f){
      var steps = f.steps || [];
      h += '<div class="card" style="padding:16px;cursor:pointer" onclick="showFunnelForm(''+f.id+'')">';
      h += '<div style="display:flex;justify-content:space-between;align-items:start;margin-bottom:8px">';
      h += '<div><div style="font-weight:600;font-size:15px;color:#1e293b">'+esc(f.name)+'</div>';
      h += '<div style="font-size:11px;color:#94a3b8">/'+esc(f.slug)+' &bull; '+steps.length+' steps</div></div>';
      h += '<span class="badge badge-'+ (f.is_active?'green':'gray') +'">'+ (f.is_active?'Active':'Draft') +'</span>';
      h += '</div>';
      h += '<div style="display:flex;align-items:center;gap:4px;padding:8px 0">';
      steps.slice(0,5).forEach(function(s,i){
        h += '<div style="flex:1;text-align:center">';
        h += '<div style="width:28px;height:28px;border-radius:50%;background:#3b82f6;color:#fff;display:flex;align-items:center;justify-content:center;margin:0 auto 2px;font-size:11px;font-weight:700">'+(i+1)+'</div>';
        h += '<div style="font-size:9px;color:#94a3b8;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">'+esc(s.card_slug)+'</div>';
        h += '</div>';
        if(i<Math.min(steps.length-1,4)) h += '<div style="width:12px;height:2px;background:#e2e8f0;flex-shrink:0;margin-top:-14px"></div>';
      });
      if(steps.length>5) h += '<div style="font-size:10px;color:#94a3b8">+'+ (steps.length-5) +' more</div>';
      h += '</div>';
      h += '</div>';
    });
    h += '</div>';
    el.innerHTML = h;
  } catch(e){
    el.innerHTML = '<div class="page-header"><div><h2>&#x1f517; Funnels</h2></div></div><div class="card" style="padding:24px;text-align:center"><p style="color:#ef4444">Failed to load: '+(e.message||'Error')+'</p></div>';
  }
}

async function showFunnelForm(editId){
  var title = 'Create Funnel';
  var data = { name:'', slug:'', steps:[] };
  var isEdit = false;
  
  if(editId){
    try {
      var f = await api('GET','/funnels/'+editId);
      title = 'Edit Funnel';
      isEdit = true;
      data = { id:f.id, name:f.name, slug:f.slug, steps: f.steps || [] };
    } catch(e){ toast('Failed to load funnel','error'); return; }
  }
  
  var cardsResp = await api('GET','/kinetic/cards');
  var cardList = Array.isArray(cardsResp) ? cardsResp : (cardsResp.cards || cardsResp.data || []);
  window.__funnelCardList = cardList;
  
  var html = '<div class="modal-header"><h2>'+esc(title)+'</h2><button class="btn-icon" onclick="closeModal()">&#x2716;</button></div>';
  html += '<div class="modal-body"><form onsubmit="return saveFunnel(event'+(isEdit?',''+data.id+''':'')+')">';
  
  html += '<div class="form-row">';
  html += '<div class="form-group"><label>Funnel Name</label><input name="name" value="'+escAttr(data.name)+'" placeholder="My Sales Funnel" required></div>';
  html += '<div class="form-group"><label>Slug (URL)</label><input name="slug" value="'+escAttr(data.slug)+'" placeholder="my-funnel" required></div>';
  html += '</div>';
  
  html += '<div class="form-section"><h3>Funnel Steps</h3><p style="font-size:11px;color:#64748b;margin:0 0 8px">Select cards for each step. Set button labels. Use external URL to link to Stripe/Gumroad checkout.</p></div>';
  html += '<div id="funnel-steps"><input type="hidden" name="steps_json" value="'+escAttr(JSON.stringify(data.steps))+'" id="funnel-steps-json">';
  html += '<div id="funnel-steps-list"></div>';
  html += '<button type="button" class="btn btn-secondary" onclick="addFunnelStep()" style="margin-top:8px;font-size:12px">+ Add Step</button>';
  html += '</div>';
  
  html += '<div class="modal-footer" style="padding:0;padding-top:14px;border-top:1px solid #e2e8f0">';
  html += '<button type="button" class="btn btn-secondary" onclick="closeModal()">Cancel</button>';
  html += '<button type="submit" class="btn btn-primary">'+(isEdit?'Update':'Create')+' Funnel</button>';
  html += '</div></form></div>';
  
  showModal(html, true);
  renderFunnelSteps(data.steps, cardList);
}

function renderFunnelSteps(steps, cardList){
  var list = document.getElementById('funnel-steps-list');
  if(!list) return;
  
  var h = '';
  if(!steps.length){
    h += '<div style="text-align:center;padding:16px;color:#94a3b8;font-size:12px;border:2px dashed #e2e8f0;border-radius:8px">No steps yet. Add a step below.</div>';
  }
  steps.forEach(function(s,i){
    h += '<div class="funnel-step-row" style="display:flex;align-items:center;gap:8px;padding:10px;background:#f8fafc;border:1px solid #e2e8f0;border-radius:8px;margin-bottom:6px" data-index="'+i+'">';
    h += '<div style="width:32px;height:32px;border-radius:50%;background:#3b82f6;color:#fff;display:flex;align-items:center;justify-content:center;font-size:14px;font-weight:700;flex-shrink:0">'+(i+1)+'</div>';
    h += '<div style="flex:1;min-width:0">';
    h += '<select onchange="updateFunnelStep('+i+',card_slug,this.value)" style="width:100%;font-size:12px;padding:6px">';
    h += '<option value="">Select card...</option>';
    cardList.forEach(function(c){
      h += '<option value="'+escAttr(c.slug)+'"'+(s.card_slug===c.slug?' selected':'')+'>'+esc(c.title||c.slug)+' ('+esc(c.template_type||'card')+')</option>';
    });
    h += '</select>';
    h += '</div>';
    h += '<input type="text" value="'+escAttr(s.button_label)+'" placeholder="Button label" onchange="updateFunnelStep('+i+',button_label,this.value)" style="width:100px;font-size:12px;padding:6px;border:1px solid #e2e8f0;border-radius:4px">';
    h += '<input type="url" value="'+escAttr(s.button_url||'')+'" placeholder="Checkout URL (opt)" onchange="updateFunnelStep('+i+',button_url,this.value)" style="width:120px;font-size:11px;padding:6px;border:1px solid #e2e8f0;border-radius:4px" title="Link to Stripe/Gumroad checkout or leave blank">';
    h += '<div style="display:flex;gap:2px;flex-shrink:0">';
    if(i>0) h += '<button type="button" class="btn-icon" onclick="moveFunnelStep('+i+',-1)" style="font-size:14px;padding:2px 4px">&#x25B2;</button>';
    if(i<steps.length-1) h += '<button type="button" class="btn-icon" onclick="moveFunnelStep('+i+',1)" style="font-size:14px;padding:2px 4px">&#x25BC;</button>';
    h += '<button type="button" class="btn-icon" onclick="removeFunnelStep('+i+')" style="font-size:14px;padding:2px 4px;color:#ef4444">&#x2716;</button>';
    h += '</div>';
    h += '</div>';
  });
  list.innerHTML = h;
}

function addFunnelStep(){
  var jsonEl = document.getElementById('funnel-steps-json');
  if(!jsonEl) return;
  var steps = JSON.parse(jsonEl.value || '[]');
  steps.push({ order: steps.length+1, card_slug:'', button_label:'Next', button_url:'' });
  jsonEl.value = JSON.stringify(steps);
  var cardList = window.__funnelCardList || [];
  renderFunnelSteps(steps, cardList);
}

function updateFunnelStep(index, field, value){
  var jsonEl = document.getElementById('funnel-steps-json');
  if(!jsonEl) return;
  var steps = JSON.parse(jsonEl.value || '[]');
  if(steps[index]) steps[index][field] = value;
  jsonEl.value = JSON.stringify(steps);
}

function moveFunnelStep(index, direction){
  var jsonEl = document.getElementById('funnel-steps-json');
  if(!jsonEl) return;
  var steps = JSON.parse(jsonEl.value || '[]');
  var newIndex = index + direction;
  if(newIndex<0 || newIndex>=steps.length) return;
  var tmp = steps[index];
  steps[index] = steps[newIndex];
  steps[newIndex] = tmp;
  steps.forEach(function(s,i){ s.order = i+1; });
  jsonEl.value = JSON.stringify(steps);
  var cardList = window.__funnelCardList || [];
  renderFunnelSteps(steps, cardList);
}

function removeFunnelStep(index){
  var jsonEl = document.getElementById('funnel-steps-json');
  if(!jsonEl) return;
  var steps = JSON.parse(jsonEl.value || '[]');
  steps.splice(index, 1);
  steps.forEach(function(s,i){ s.order = i+1; });
  jsonEl.value = JSON.stringify(steps);
  var cardList = window.__funnelCardList || [];
  renderFunnelSteps(steps, cardList);
}

async function saveFunnel(e, editId){
  e.preventDefault();
  var fd = e.target.elements;
  var steps = JSON.parse(fd['steps_json'].value || '[]');
  
  if(!steps.length){ toast('Add at least one step','error'); return; }
  
  var data = {
    name: fd['name'].value,
    slug: fd['slug'].value,
    steps: steps
  };
  
  try {
    if(editId){
      await api('PUT','/funnels/'+editId, data);
      toast('Funnel updated');
    } else {
      var result = await api('POST','/funnels', data);
      toast('Funnel created! URL: '+(result.public_url||''));
    }
    closeModal();
    S.view = 'funnels';
    renderView();
  } catch(e){
    toast(e.message||'Error','error');
  }
}

