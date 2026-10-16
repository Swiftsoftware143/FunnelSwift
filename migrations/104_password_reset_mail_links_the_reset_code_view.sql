-- 104_password_reset_mail_links_the_reset_code_view.sql
--
-- kanban cron `default`, 2026-10-16. FunnelSwift's password-reset mail was the last one in the
-- fleet that handed over a code with no way to USE it.
--
-- MEASURED: the row this file updates is the message actually sent (src/email.rs::render_fallback
-- is only reached when no default row exists). It read
--
--   Open {{login_url}} and click "Forgot password?", then enter the code above to choose a new
--   password.
--
-- and that instruction cannot work in this app. `{{login_url}}` is https://app.funnelswift.net/login,
-- whose only route to the code field is the "Forgot password?" link -> "Send reset code", which
-- POSTs /api/v1/auth/forgot-password. That handler does
--
--   UPDATE password_resets SET used = true WHERE user_id = $1 AND used = false
--
-- (src/auth/handlers.rs, forgot_password) BEFORE minting a new token — so pressing the one button the
-- mail tells the customer to press marks the code they are holding USED. They paste the code they
-- were reading, get "Invalid or expired reset token", and only a SECOND mail's code works.
--
-- The fix is the one the sibling apps already carry (IncentiveSwift
-- zz_is14_password_reset_mail_links_the_reset_page.sql, WorkflowSwift 090): the mail links STRAIGHT
-- to the code view with the code filled in, so no second code is ever needed and no POST has to
-- happen first. FunnelSwift's code view lives on the login page, so the link is
-- `<login_url>?token=<code>` — www-app/login.html reads `?token=` and opens the reset view with the
-- field prefilled (no query param -> the ordinary sign-in path is byte-identical).
--
-- Placeholders are exactly the ones the senders bind: {{name}}, {{app_name}}, {{token}},
-- {{login_url}}. Expiry text stays "1 hour" — this app mints `Utc::now() + Duration::hours(1)`.
-- Nothing else about the send changes; this migration sends nothing.
--
-- Idempotent by construction: the same UPDATE on the same row yields the same values. It targets the
-- GLOBAL default row only (aid IS NULL), so an operator's own edited per-tenant template is left
-- alone.

UPDATE email_templates
   SET body = $fs_reset_body$Hi {{name}},

We received a request to reset the password on your {{app_name}} account.

Your reset code is:

{{token}}

Open {{login_url}}?token={{token}} to choose a new password - the code is filled in for you. If that link does not open the form, open {{login_url}} and paste the code above.

This code expires in 1 hour, so use it soon.

If you didn't ask for a password reset, you don't need to do anything - your password stays the same and no changes have been made to your account.

- The FunnelSwift Team$fs_reset_body$,
       html_body = $fs_reset_html$<h2 style="margin:0 0 12px;font-size:18px;">Reset your password</h2>
<p style="margin:0 0 12px;">Hi {{name}},</p>
<p style="margin:0 0 12px;">We received a request to reset the password on your {{app_name}} account.</p>
<p style="margin:0 0 12px;font-size:20px;"><code>{{token}}</code></p>
<p style="margin:0 0 12px;">Open <a href="{{login_url}}?token={{token}}">{{login_url}}</a> and choose a new password - the reset code is filled in for you.</p>
<p style="margin:0 0 12px;">If that link does not open the form, go to <strong>{{login_url}}</strong> and paste the code above.</p>
<p style="margin:0 0 12px;">This code expires in 1 hour, so use it soon.</p>
<p style="margin:0 0 12px;">If you didn't ask for a password reset, you don't need to do anything - your password stays the same and no changes have been made to your account.</p>
<p style="margin:0;">- The FunnelSwift Team</p>$fs_reset_html$,
       updated_at = NOW()
 WHERE template_type = 'password_reset'
   AND aid IS NULL
   AND is_default = true;
