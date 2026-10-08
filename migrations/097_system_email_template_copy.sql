-- 097_system_email_template_copy.sql
--
-- kanban t_2f9a6576 (system email templates: skilled copy + real merge fields + panel-editable).
--
-- Copied from the Claude-authored brief (/tmp/fsw-tpl.json generation step, validated against the
-- merge fields each sender binds in src/email.rs).
--
-- WHY A NEW MIGRATION AND NOT AN EDIT: 074 is already applied and checksummed.
-- Every insert is guarded by NOT EXISTS on the type's global default row, so re-running on an
-- install that already has the row is a no-op and never clobbers copy an operator wrote.

-- 1. credentials — the ONE mail this app sends on its own at signup (and when an admin creates a
--    user): the generated first password. It had NO row at all, so its copy lived in the Rust
--    inline fallback and was uneditable from the admin panel. Seating the default row is what
--    makes it panel-editable (the send path prefers a row and falls back to the inline arm).

-- credentials: New Account - Login Details
INSERT INTO email_templates (template_type, name, subject, body, html_body, is_default)
SELECT 'credentials', $tpl$New Account - Login Details$tpl$, $tpl$Your {{app_name}} account is ready$tpl$, $tpl$Hi {{name}},

Your {{app_name}} account has been created and is ready to use.

Sign in at {{login_url}} with:
Email: {{email}}
Password: {{password}}

The password above is temporary. After you sign in, go to your profile settings and replace it with one of your own.

If you weren't expecting this email, someone may have entered your address by mistake - contact support and we'll take a look.

- The FunnelSwift Team$tpl$, $tpl$<h2 style="margin:0 0 12px;font-size:18px;">Your account is ready</h2>
<p style="margin:0 0 12px;">Hi {{name}},</p>
<p style="margin:0 0 12px;">Your {{app_name}} account has been created and is ready to use.</p>
<p style="margin:0 0 4px;">Sign in at <a href="{{login_url}}" style="color:#2563eb;">{{login_url}}</a> with:</p>
<ul style="margin:0 0 12px;padding-left:20px;">
<li>Email: <strong>{{email}}</strong></li>
<li>Password: <code>{{password}}</code></li>
</ul>
<p style="margin:0 0 12px;">The password above is temporary. After you sign in, go to your profile settings and replace it with one of your own.</p>
<p style="margin:0 0 12px;">If you weren't expecting this email, someone may have entered your address by mistake - contact support and we'll take a look.</p>
<p style="margin:0;">- The FunnelSwift Team</p>$tpl$, true
WHERE NOT EXISTS (
    SELECT 1 FROM email_templates WHERE template_type = 'credentials' AND aid IS NULL AND is_default
);

-- password_reset: Password Reset Code
INSERT INTO email_templates (template_type, name, subject, body, html_body, is_default)
SELECT 'password_reset', $tpl$Password Reset Code$tpl$, $tpl$Your password reset code for {{app_name}}$tpl$, $tpl$Hi {{name}},

We received a request to reset the password on your {{app_name}} account. Use this code to finish:

{{token}}

This code expires in 1 hour, so use it soon.

If you didn't ask for a password reset, you don't need to do anything - your password stays the same and no changes have been made to your account.

- The FunnelSwift Team$tpl$, $tpl$<h2 style="margin:0 0 12px;font-size:18px;">Reset your password</h2>
<p style="margin:0 0 12px;">Hi {{name}},</p>
<p style="margin:0 0 12px;">We received a request to reset the password on your {{app_name}} account. Use this code to finish:</p>
<p style="margin:0 0 12px;font-size:20px;"><code>{{token}}</code></p>
<p style="margin:0 0 12px;">This code expires in 1 hour, so use it soon.</p>
<p style="margin:0 0 12px;">If you didn't ask for a password reset, you don't need to do anything - your password stays the same and no changes have been made to your account.</p>
<p style="margin:0;">- The FunnelSwift Team</p>$tpl$, true
WHERE NOT EXISTS (
    SELECT 1 FROM email_templates WHERE template_type = 'password_reset' AND aid IS NULL AND is_default
);

-- welcome: Post-Signup Welcome
INSERT INTO email_templates (template_type, name, subject, body, html_body, is_default)
SELECT 'welcome', $tpl$Post-Signup Welcome$tpl$, $tpl$Welcome to {{app_name}} - let's build your first funnel$tpl$, $tpl$Hi {{name}},

Welcome to {{app_name}}. It's built for one job: capture leads with a shareable funnel page, live in minutes.

A few things to do first:
- Sign in at {{login_url}} with {{email}}
- Create your first funnel page from a template
- Add your offer and a lead capture form
- Share the page link and start collecting leads

That's it - no design skills needed.

- The FunnelSwift Team$tpl$, $tpl$<h2 style="margin:0 0 12px;font-size:18px;">Welcome to {{app_name}}</h2>
<p style="margin:0 0 12px;">Hi {{name}},</p>
<p style="margin:0 0 12px;">{{app_name}} is built for one job: capture leads with a shareable funnel page, live in minutes.</p>
<p style="margin:0 0 4px;">A few things to do first:</p>
<ul style="margin:0 0 12px;padding-left:20px;">
<li>Sign in at <a href="{{login_url}}" style="color:#2563eb;">{{login_url}}</a> with {{email}}</li>
<li>Create your first funnel page from a template</li>
<li>Add your offer and a lead capture form</li>
<li>Share the page link and start collecting leads</li>
</ul>
<p style="margin:0 0 12px;">That's it - no design skills needed.</p>
<p style="margin:0;">- The FunnelSwift Team</p>$tpl$, true
WHERE NOT EXISTS (
    SELECT 1 FROM email_templates WHERE template_type = 'welcome' AND aid IS NULL AND is_default
);

-- purchase_confirmed: Purchase Confirmation Receipt
INSERT INTO email_templates (template_type, name, subject, body, html_body, is_default)
SELECT 'purchase_confirmed', $tpl$Purchase Confirmation Receipt$tpl$, $tpl$You're on the {{plan_name}} plan$tpl$, $tpl$Hi {{name}},

Your payment for the {{plan_name}} plan has gone through, and your {{app_name}} account is now active.

You can sign in any time at {{login_url}} and pick up right where you left off - all your funnels and settings are right there waiting.

Thanks for upgrading.

- The FunnelSwift Team$tpl$, $tpl$<h2 style="margin:0 0 12px;font-size:18px;">Payment confirmed</h2>
<p style="margin:0 0 12px;">Hi {{name}},</p>
<p style="margin:0 0 12px;">Your payment for the <strong>{{plan_name}}</strong> plan has gone through, and your {{app_name}} account is now active.</p>
<p style="margin:0 0 12px;">You can sign in any time at <a href="{{login_url}}" style="color:#2563eb;">{{login_url}}</a> and pick up right where you left off - all your funnels and settings are right there waiting.</p>
<p style="margin:0 0 12px;">Thanks for upgrading.</p>
<p style="margin:0;">- The FunnelSwift Team</p>$tpl$, true
WHERE NOT EXISTS (
    SELECT 1 FROM email_templates WHERE template_type = 'purchase_confirmed' AND aid IS NULL AND is_default
);

-- 2. welcome — the row 074 seeded was 173 chars of placeholder and never sent. Rewrite the copy,
--    but ONLY while the row still carries the seed wording: if an operator has written their own,
--    leave it alone.
UPDATE email_templates
   SET name = $tpl$Post-Signup Welcome$tpl$,
       subject = $tpl$Welcome to {{app_name}} - let's build your first funnel$tpl$,
       body = $tpl$Hi {{name}},

Welcome to {{app_name}}. It's built for one job: capture leads with a shareable funnel page, live in minutes.

A few things to do first:
- Sign in at {{login_url}} with {{email}}
- Create your first funnel page from a template
- Add your offer and a lead capture form
- Share the page link and start collecting leads

That's it - no design skills needed.

- The FunnelSwift Team$tpl$,
       html_body = $tpl$<h2 style="margin:0 0 12px;font-size:18px;">Welcome to {{app_name}}</h2>
<p style="margin:0 0 12px;">Hi {{name}},</p>
<p style="margin:0 0 12px;">{{app_name}} is built for one job: capture leads with a shareable funnel page, live in minutes.</p>
<p style="margin:0 0 4px;">A few things to do first:</p>
<ul style="margin:0 0 12px;padding-left:20px;">
<li>Sign in at <a href="{{login_url}}" style="color:#2563eb;">{{login_url}}</a> with {{email}}</li>
<li>Create your first funnel page from a template</li>
<li>Add your offer and a lead capture form</li>
<li>Share the page link and start collecting leads</li>
</ul>
<p style="margin:0 0 12px;">That's it - no design skills needed.</p>
<p style="margin:0;">- The FunnelSwift Team</p>$tpl$,
       updated_at = NOW()
 WHERE template_type = 'welcome'
   AND aid IS NULL
   AND is_default
   AND body LIKE '%Your account has been created successfully.%';
