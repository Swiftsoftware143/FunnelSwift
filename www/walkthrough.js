// ===== Walkthrough System =====
(function() {
  // Don't run on login page
  if (window.location.pathname === '/' || window.location.pathname === '/index.html') return;
  var wtText = {
    dashboard: 'Welcome to FunnelSwift! Your lead generation hub. Track total leads, conversions, top sources, and affiliate performance. Click any stat to drill down.',
    leads: 'Manage your prospect leads. Add leads manually with name, email, phone, and source. Assign tags and route to affiliates or keep internal.',
    affiliates: 'Manage affiliate partners. Each gets a unique auto-generated ID. Configure commission rates, track payouts, and route leads to their systems.',
    plans: 'Configure subscription plans and pricing. Plans define feature access, credit limits, and commission structures for affiliates.',
    tags: 'Organize leads with tag groups: Source, Status, Events, Services, Engagement, Custom. Tags auto-sync from plan mappings and are tenant-specific.',
    tagGroups: 'Organize leads with tag groups: Source, Status, Events, Services, Engagement, Custom. Tags auto-sync from plan mappings and are tenant-specific.',
    apiKeys: 'Generate API keys for programmatic lead ingestion. Keys can be revoked individually. Integrate via webhooks and HTTP API.',
    settings: 'Configure your account, change password, and manage tenant preferences.',
    webhooks: 'Configure webhooks to receive real-time lead and event notifications. Test and manage your integrations here.',
    webToLead: 'Embed a JavaScript snippet on your website to auto-capture leads from any form. Assign one or more tags for automatic labeling. Leads appear in your workspace instantly.',
    targetSoftware: 'Manage target software integrations for routing leads to external platforms.',
    planTagMappings: 'Map plans to tags to control feature access and commission rules per plan.'
  };
  function getPageName(view) {
    return view || 'dashboard';
  }
  function globalShowWalkthrough(page) {
    var p = page || getPageName(typeof S !== 'undefined' && S && S.view);
    var key = 'ws_funnelswift_' + p;
    if (localStorage.getItem(key)) return;
    var text = wtText[p];
    if (!text) text = wtText.dashboard;
    if (document.getElementById('wt-overlay')) {
      document.getElementById('wt-text').textContent = text;
      return;
    }
    var html = '<div id="wt-overlay" style="position:fixed;inset:0;background:rgba(0,0,0,0.55);z-index:2000;display:flex;align-items:center;justify-content:center;padding:20px;">';
    html += '<div id="wt-modal" style="background:#fff;border-radius:12px;max-width:400px;width:100%;box-shadow:0 20px 60px rgba(0,0,0,0.25);animation:fadeIn .25s ease-out;overflow:hidden;">';
    html += '<div style="padding:24px 24px 8px;">';
    html += '<div style="width:40px;height:40px;border-radius:50%;background:#6366f1;display:flex;align-items:center;justify-content:center;margin-bottom:12px;font-size:18px;color:#fff;font-weight:bold;">?</div>';
    html += '<h3 style="font-size:18px;font-weight:600;color:#1e293b;margin-bottom:6px;">Did you know?</h3>';
    html += '<p id="wt-text" style="font-size:14px;color:#64748b;line-height:1.6;"></p>';
    html += '</div>';
    html += '<div style="padding:12px 24px 20px;display:flex;align-items:center;justify-content:space-between;">';
    html += '<button id="wt-remind" style="background:none;border:none;color:#94a3b8;font-size:13px;cursor:pointer;text-decoration:underline;padding:6px;">Remind Later</button>';
    html += '<button id="wt-gotit" style="background:#6366f1;color:#fff;border:none;border-radius:8px;padding:8px 20px;font-size:14px;font-weight:500;cursor:pointer;">Got it!</button>';
    html += '</div>';
    html += '</div></div>';
    document.body.insertAdjacentHTML('beforeend', html);
    var overlay = document.getElementById('wt-overlay');
    document.getElementById('wt-text').textContent = text;
    document.getElementById('wt-gotit').addEventListener('click', function() {
      localStorage.setItem(key, '1');
      overlay.remove();
    });
    document.getElementById('wt-remind').addEventListener('click', function() {
      overlay.remove();
    });
    overlay.addEventListener('click', function(e) {
      if (e.target === overlay) overlay.remove();
    });
  }
  // Help icon
  var helpIcon = document.createElement('div');
  helpIcon.id = 'wt-help';
  helpIcon.innerHTML = '?';
  helpIcon.style.cssText = 'position:fixed;bottom:20px;right:20px;width:40px;height:40px;border-radius:50%;background:#6366f1;color:#fff;display:flex;align-items:center;justify-content:center;font-size:18px;font-weight:bold;cursor:pointer;z-index:2001;box-shadow:0 2px 8px rgba(99,102,241,0.4);transition:transform .15s;';
  helpIcon.addEventListener('mouseenter', function(){this.style.transform='scale(1.1)';});
  helpIcon.addEventListener('mouseleave', function(){this.style.transform='scale(1)';});
  helpIcon.addEventListener('click', function() {
    globalShowWalkthrough(typeof S !== 'undefined' && S && S.view);
  });
  // Watch for app container to be present before appending help icon
  var appCheck = setInterval(function() {
    if (document.getElementById('app')) {
      document.body.appendChild(helpIcon);
      clearInterval(appCheck);
    }
  }, 100);
  // Patch navigate
  var origNavigate = window.navigate;
  window.navigate = function(v) {
    if (origNavigate) origNavigate(v);
    setTimeout(function() {
      globalShowWalkthrough(v);
    }, 300);
  };
  // Auto-show on page load (after render completes)
  setTimeout(function() {
    globalShowWalkthrough();
  }, 500);
  // Expose for help icon
  window.showWalkthrough = globalShowWalkthrough;
})();
