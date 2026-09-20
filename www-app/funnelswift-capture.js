/**
 * FunnelSwift Web-to-Lead Capture Script v2.0
 * Embed on any website to capture leads directly into FunnelSwift.
 *
 * Usage — OPTION A: Paste in <head> (with window config):
 *   <script>
 *     window.FunnelSwiftConfig = { configId: "YOUR_CONFIG_ID" };
 *   </script>
 *   <script src="https://funnelswift.net/funnelswift-capture.js" defer></script>
 *
 * Usage — OPTION B: Paste before </body> (self-contained, easiest):
 *   <script src="https://funnelswift.net/funnelswift-capture.js" defer
 *     data-config-id="YOUR_CONFIG_ID"
 *     data-public-key="YOUR_PUBLIC_KEY"></script>
 *
 * Both options auto-capture all forms on the page without manual API key handling.
 */
(function() {
  'use strict';

  // ── Read config from window var OR data attributes on this script tag ──
  var cfg = window.FunnelSwiftConfig || {};
  var apiKey = cfg.apiKey || null;
  var configId = cfg.configId || null;
  var endpoint = cfg.endpoint || 'https://funnelswift.net/api/v1/web-to-lead';

  // Try data attributes from the script tag (Option B — easiest drop-in)
  if (!configId) {
    var scripts = document.getElementsByTagName('script');
    for (var i = 0; i < scripts.length; i++) {
      var s = scripts[i];
      if (s.src && s.src.indexOf('funnelswift-capture.js') !== -1) {
        if (s.getAttribute('data-config-id')) configId = s.getAttribute('data-config-id');
        if (s.getAttribute('data-public-key')) apiKey = s.getAttribute('data-public-key');
        if (s.getAttribute('data-endpoint')) endpoint = s.getAttribute('data-endpoint');
        break;
      }
    }
  }

  if (!configId) {
    console.warn('[FunnelSwift] No configId found. Set via FunnelSwiftConfig.configId or data-config-id attribute.');
    return;
  }

  // ── Extract field values from any form ──
  function getFormData(form) {
    var data = {};
    var fields = form.querySelectorAll('input, select, textarea');
    for (var i = 0; i < fields.length; i++) {
      var f = fields[i];
      if (!f.name || f.disabled || f.type === 'submit' || f.type === 'button' || f.type === 'file') continue;
      if (f.type === 'checkbox') {
        if (f.checked) {
          data[f.name] = data[f.name] || [];
          data[f.name].push(f.value);
        }
        continue;
      }
      if (f.type === 'radio') {
        if (f.checked) data[f.name] = f.value;
        continue;
      }
      data[f.name] = f.value;
    }
    return data;
  }

  // ── Map common field names to FunnelSwift fields ──
  function mapFields(formData) {
    var params = { config_id: configId };
    // Use public_key as api_key for auth, or fall through if apiKey was set explicitly
    if (apiKey) params.api_key = apiKey;

    var fieldMap = {
      'name': ['name', 'full_name', 'fullname'],
      'first_name': ['first_name', 'firstname', 'fname', 'firstName'],
      'last_name': ['last_name', 'lastname', 'lname', 'surname', 'lastName'],
      'email': ['email', 'e-mail', 'email_address', 'emailaddress'],
      'phone': ['phone', 'telephone', 'tel', 'mobile', 'phone_number'],
      'company': ['company', 'organization', 'org', 'business'],
      'notes': ['notes', 'message', 'comments', 'description']
    };

    for (var key in fieldMap) {
      var aliases = fieldMap[key];
      for (var a = 0; a < aliases.length; a++) {
        if (formData[aliases[a]] !== undefined) {
          params[key] = formData[aliases[a]];
          delete formData[aliases[a]];
          break;
        }
      }
    }

    // Everything else lands in `extra`
    params.extra = {};
    for (var k in formData) {
      if (Object.prototype.hasOwnProperty.call(formData, k)) {
        params.extra[k] = formData[k];
      }
    }

    return params;
  }

  // ── POST captured lead to FunnelSwift API ──
  function submitToFunnelSwift(payload) {
    var xhr = new XMLHttpRequest();
    xhr.open('POST', endpoint, true);
    xhr.setRequestHeader('Content-Type', 'application/json');
    xhr.onload = function() {
      if (xhr.status >= 200 && xhr.status < 300) {
        console.log('[FunnelSwift] Lead captured successfully');
        document.dispatchEvent(new CustomEvent('funnelswift:leadCaptured', {
          detail: JSON.parse(xhr.responseText)
        }));
      } else {
        console.warn('[FunnelSwift] Lead capture failed:', xhr.status, xhr.responseText);
        document.dispatchEvent(new CustomEvent('funnelswift:error', {
          detail: { status: xhr.status, response: xhr.responseText }
        }));
      }
    };
    xhr.onerror = function() {
      console.error('[FunnelSwift] Network error');
      document.dispatchEvent(new CustomEvent('funnelswift:error', {
        detail: { status: 0, response: 'Network error' }
      }));
    };
    xhr.send(JSON.stringify(payload));
  }

  // ── Hook a form to capture on submit ──
  function hookForm(form) {
    if (form.dataset.funnelswiftHooked) return;
    form.dataset.funnelswiftHooked = '1';

    var originalSubmit = form.onsubmit;

    form.addEventListener('submit', function(e) {
      var formData = getFormData(form);
      var payload = mapFields(formData);

      // Only send if there's a name or email
      if (payload.name || payload.email || payload.first_name) {
        submitToFunnelSwift(payload);
      }

      // Don't prevent default — let the form submit normally
      if (typeof originalSubmit === 'function') {
        return originalSubmit.call(form, e);
      }
    });
  }

  // ── Initialize — hook existing and future forms ──
  function init() {
    var forms = document.querySelectorAll('form');
    for (var i = 0; i < forms.length; i++) {
      hookForm(forms[i]);
    }

    // Watch for dynamically added forms
    if (window.MutationObserver) {
      var observer = new MutationObserver(function(mutations) {
        for (var m = 0; m < mutations.length; m++) {
          var added = mutations[m].addedNodes;
          for (var n = 0; n < added.length; n++) {
            if (added[n].tagName === 'FORM') hookForm(added[n]);
            if (added[n].querySelectorAll) {
              var childForms = added[n].querySelectorAll('form');
              for (var f = 0; f < childForms.length; f++) hookForm(childForms[f]);
            }
          }
        }
      });
      observer.observe(document.body, { childList: true, subtree: true });
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }

  // ── Expose public API for programmatic capture ──
  window.FunnelSwift = {
    capture: function(data) {
      if (apiKey) data.api_key = apiKey;
      if (configId && !data.config_id) data.config_id = configId;
      submitToFunnelSwift(data);
    },
    getConfig: function() {
      return { apiKey: apiKey, configId: configId, endpoint: endpoint };
    }
  };
})();
