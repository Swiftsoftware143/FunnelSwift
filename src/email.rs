//! Email module — sends transactional emails using database-stored templates.
//!
//! Templates are stored in `email_templates` with html_body (HTML) and body (plain text).
//! Falls back to hardcoded inline templates when DB template not found.
//!
//! Provider resolution is **DB only** (`crate::email_provider`) — no env vars:
//!   - `send_template_email_for_tenant` resolves the tenant's Mailgun/SMTP/email config from
//!     `tenant_settings` (`email_config` / `mailgun_config` / `smtp_config`), then falls back to
//!     the global `admin_settings.email` row (system mail).
//!   - `send_template_email` is the system path (no tenant context) — same global row.
//!   - Any other provider choice (smtp | mailgun | sendgrid | sendiio) is stored in that row.
//!
//! Unconfigured is never fatal: it is logged and the send is skipped.
//!
//! Direct API send (no queue).
//!
//! Placeholder contract: **`{{key}}` only** (double braces). A body written with single
//! braces (`{name}`) is not a placeholder and reaches the recipient verbatim, so `render`
//! reports every leftover placeholder-shaped token instead of failing silently. Every key a
//! sender binds must be one of the merge fields the admin UI lists for that type
//! (`GET /api/v1/admin/email-templates/types`) — see `bound_vars` below.

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Product name, base URL and login URL — the URL-shaped merge fields the admin panel
/// advertises for `welcome` / `password_reset` / `purchase_confirmed`
/// (`app_name`, `login_url`; `app_url` is kept for the inline fallbacks below).
/// kanban t_2349e6ce: the types endpoint advertised `app_name`/`login_url` while the senders
/// bound only `app_url`, so an admin-authored `{{app_name}}` sent literally.
const APP_NAME: &str = "FunnelSwift";
const APP_URL: &str = "https://app.funnelswift.net";
const APP_LOGIN_URL: &str = "https://app.funnelswift.net/login";

/// The app's OWN wordmark on this app's own host, used for the header of every message sent by a
/// tenant that has configured no branding (2026-10-24).
///
/// A mail client fetches it over the network with no session, so it must be an absolute URL on a
/// host that answers publicly — and it must be a PNG: Gmail and Outlook do not render SVG, so the
/// `/logo.svg` this app serves for its pages would arrive as a blank box. `/logo.png` is that
/// file's raster twin (400x80, same drawing), served by both the app and the marketing hosts.
const APP_LOGO_PATH: &str = "/logo.png";

/// The identity every message falls back to when its tenant has configured none: the app's own
/// name and wordmark, in the fleet's default accent.
///
/// Before this the header block only appeared once a tenant had uploaded a logo of their own, so
/// the credentials mail a BRAND-NEW customer receives — the first thing this product ever shows
/// them — arrived with no wordmark at all.
fn app_default_branding() -> crate::branding::Branding {
    crate::branding::Branding {
        brand_name: APP_NAME.to_string(),
        brand_color: String::new(),
        logo_url: APP_LOGO_PATH.to_string(),
    }
}

/// The variables every template type resolves: the advertised merge fields plus `app_url`.
/// Bind these on every send path so no advertised field can ever go out as literal text.
fn bound_vars<'a>(
    mut vars: std::collections::HashMap<&'a str, &'a str>,
) -> std::collections::HashMap<&'a str, &'a str> {
    vars.insert("app_name", APP_NAME);
    vars.insert("app_url", APP_URL);
    vars.insert("login_url", APP_LOGIN_URL);
    // Tenant branding (kanban t_c06a32eb) — DEFAULTS only: `render_template` overwrites these with
    // the account's own values when the account has branding. Binding them here is what keeps the
    // `bound_vars_cover_every_advertised_merge_field` contract true: an advertised field that no
    // sender binds goes out as literal text.
    vars.insert("brand_name", APP_NAME);
    vars.insert("logo_url", "");
    vars
}

/// Placeholder-shaped tokens that survived rendering — i.e. keys no sender bound.
/// `{name}` and `{{name}}` are both reported as `name`.
fn unsubstituted(rendered: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = rendered;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = after[..close].trim_matches('{').trim_matches('}').trim();
        if !name.is_empty()
            && name.len() < 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            && !out.contains(&name)
        {
            out.push(name);
        }
        rest = &after[close + 1..];
    }
    out
}

/// Render a template string by replacing {{key}} placeholders. Anything left over is
/// logged: a half-substituted email is a defect, not a silent default.
fn render(template: &str, vars: &std::collections::HashMap<&str, &str>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{}}}}}", key), value);
    }
    let missing = unsubstituted(&result);
    if !missing.is_empty() {
        tracing::warn!(
            placeholder = %missing.join(","),
            "email template placeholder(s) left unsubstituted — the recipient would see them literally. \
             Use the double-brace form and only the merge fields GET /api/v1/admin/email-templates/types advertises"
        );
    }
    result
}

/// Send an already-rendered email through the tenant's DB-configured provider.
/// Graceful degradation: unconfigured logs a warning and skips (never panics, never
/// silently uses a server-wide env credential).
async fn dispatch_for_tenant(
    pool: &PgPool,
    tenant_id: Option<Uuid>,
    to: &str,
    subject: &str,
    body: Option<&str>,
    html: Option<&str>,
) -> Result<(), String> {
    let Some(cfg) = crate::email_provider::resolve(pool, tenant_id).await else {
        tracing::warn!(
            tenant = ?tenant_id,
            to = %to,
            subject = %subject,
            "email skipped — no email provider configured for this tenant (Admin > Settings > Email Provider)"
        );
        // An unconfigured provider is a FAILED send, not a silent skip (kanban t_499119d2): this was
        // the one arm of the funnel that returned before recording anything, so the failure never
        // reached `email_send_log` and the operator's delivery history simply had no row for it.
        // Record it on the same slots every other failure writes.
        record_last_send(
            pool,
            to,
            subject,
            false,
            "email provider not configured (Admin > Settings > Email Provider)",
        )
        .await;
        return Err(
            "Email provider not configured. Set it in Admin > Settings > Email Provider."
                .to_string(),
        );
    };

    let outcome = crate::email_provider::deliver(&cfg, to, subject, body.unwrap_or(""), html).await;
    // `deliver` answers Ok(()) for a reserved/harness recipient — it deliberately attempts nothing —
    // so Ok alone is not a delivery. Recording that as accepted is the misreport this line exists to
    // prevent; the withheld case is recorded as the non-send it is (2026-10-14, same class fixed on
    // ADASwift and MissedCall the same day).
    match (&outcome, crate::security::probe_addr::harness_domain(to)) {
        (Ok(()), Some(domain)) => {
            tracing::info!(to = %to, domain = %domain, "send withheld — recording a non-send");
            // THE SUBJECT, not a generic label (2026-11-06). This arm used to record every
            // withheld send as "transactional email (withheld)", which threw away the one fact the
            // operator needs when a real mail is suppressed by mistake: WHICH mail it was. Callers
            // that suppress at a handler level already pass a meaningful kind ("signup credentials
            // email"), so the fan-out path was the odd one out. `subject` is now the kind, and the
            // history reads "<the subject> (withheld)".
            record_withheld(
                pool,
                subject,
                to,
                &format!("'{domain}' is a fleet-harness / reserved address"),
            )
            .await;
        }
        (Ok(()), None) => record_last_send(pool, to, subject, true, "").await,
        (Err(e), _) => {
            tracing::warn!(provider = %cfg.provider, to = %to, "email send failed: {e}");
            record_last_send(pool, to, subject, false, e).await;
        }
    }
    outcome
}

/// The operator's view of the LAST transactional send this installation attempted.
///
/// The send funnel already LOGS a failure, but a log is not a panel: this records the outcome under
/// `admin_settings.email_last_send`, which `GET /api/v1/admin/email-config` returns, so an operator
/// can see that the credentials mail of a real signup was REFUSED without grepping the logs. Five of
/// the six apps already report their last send (ADASwift `email_send_state`, MissedCall
/// `email_last_send`, WorkflowSwift `last_send_ok`); FunnelSwift recorded nothing at all.
///
/// Writing the record NEVER fails a send: an error here is logged and dropped.
///
/// Two records, one funnel: the single-value panel slot below AND one row of history. The slot
/// answers "what happened to the most recent message?"; the history (migration 109) answers "did that
/// customer's welcome mail ever go out?", which the slot cannot — any later mail overwrites it. An
/// operator who watched a signup's credentials mail fail saw the failure erased by the next password
/// reset, so the one screen meant to prove delivery proved nothing.
pub(crate) async fn record_last_send(
    pool: &PgPool,
    to: &str,
    subject: &str,
    ok: bool,
    detail: &str,
) {
    write_panel(pool, to, subject, ok, detail).await;
    append_log(
        pool,
        to,
        subject,
        if ok { "sent" } else { "failed" },
        detail,
    )
    .await;
}

/// Append one row to the delivery history (`email_send_log`, migration 109).
async fn append_log(pool: &PgPool, to: &str, subject: &str, status: &str, detail: &str) {
    if let Err(e) = sqlx::query(
        "INSERT INTO email_send_log (to_email, subject, status, detail) VALUES ($1, $2, $3, $4)",
    )
    .bind(to)
    .bind(subject)
    .bind(status)
    .bind(detail.chars().take(240).collect::<String>())
    .execute(pool)
    .await
    {
        tracing::warn!("could not append to email_send_log: {e}");
    }
}

/// Write the one-slot record the Email Provider panel reads (`admin_settings.email_last_send`).
async fn write_panel(pool: &PgPool, to: &str, subject: &str, ok: bool, detail: &str) {
    let payload = json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "ok": ok,
        "to": to,
        "subject": subject,
        "detail": detail.chars().take(240).collect::<String>(),
    });
    if let Err(e) = sqlx::query(
        "INSERT INTO admin_settings (key, value, description, updated_at) VALUES ('email_last_send', $1, 'The last transactional email this installation attempted to send (written by the send funnel)', NOW()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(payload)
    .execute(pool)
    .await
    {
        tracing::warn!("could not record email_last_send: {e}");
    }
}

/// Record a message the transport REFUSED to attempt, so the operator's "last email" line reflects
/// it instead of a stale, earlier send.
///
/// `kind` names what the message was ("signup credentials email", "password reset email",
/// "transactional email"), because at a handler-level suppression the template has not been rendered
/// yet and there is no subject to record.
pub(crate) async fn record_withheld(pool: &PgPool, kind: &str, to: &str, reason: &str) {
    let detail = format!("withheld: {reason}, so nothing was sent (no provider request)");
    let subject = format!("{kind} (withheld)");
    // History FIRST and as its own status: a reserved recipient is a NON-SEND, never a provider
    // refusal, so it neither reads as a failure nor adds a second `failed` row via the slot writer.
    append_log(pool, to, &subject, "withheld", &detail).await;
    write_panel(pool, to, &subject, false, &detail).await;
}

/// The app's OWN support channel. David (2026-10-08): the reply-to/support line in a
/// transactional mail is ALWAYS the app's own — never the platform's or a sibling company's.
///
/// It is a PAGE, not a mailbox (measured 2026-11-06 on dns.google and cloudflare-dns.com): every
/// address this product named — `support@swiftsoftware.net`, `support@funnelswift.net` — lived on a
/// domain publishing **MX = 0** with nothing on this host listening on port 25, so a customer's
/// reply bounced and a locked-out customer had no way back in. The app now serves a contact form on
/// its own host that files the message straight into the operator's support queue
/// (`POST /api/v1/public/support-request`), so the advice a customer is given actually reaches a
/// person. Do not put a `support@` address back here until its domain publishes a mail exchanger.
pub const SUPPORT_CONTACT: &str = "https://app.funnelswift.net/contact.html";

/// Append the support line to a rendered transactional message.
///
/// Applied in [`render_template`] rather than to one body literal, so a body that comes
/// from the `email_templates` row (the authoritative source for these sends) carries the
/// address exactly like an inline fallback does, and a template added later inherits it.
/// Idempotent: a body that already carries the address is returned untouched.
///
/// `pub(crate)` (kanban t_f1931c05) so the admin "Send test email" route — which builds its
/// body inline and never touches `render_template` — can put the SAME footer on the message
/// an admin inspects, instead of a second, quietly drifting copy of the wording.
pub(crate) fn with_support_footer(
    text: Option<String>,
    html: Option<String>,
) -> (Option<String>, Option<String>) {
    let text = text.map(|t| {
        if t.contains(SUPPORT_CONTACT) {
            t
        } else {
            format!(
                "{}\n\nNeed help? Open {} and send us a message — no sign-in needed.\n",
                t, SUPPORT_CONTACT
            )
        }
    });
    let html = html.map(|h| {
        if h.contains(SUPPORT_CONTACT) {
            h
        } else {
            format!(
                "{}\n<p style=\"font-size:13px;color:#6b7280;text-align:center;\">Need help? <a href=\"{}\">Send us a message</a> — no sign-in needed.</p>",
                h, SUPPORT_CONTACT
            )
        }
    });
    (text, html)
}

/// Put the tenant's branding at the TOP of a rendered message (kanban t_c06a32eb).
///
/// The HTML part opens with the logo / brand-name header block; the plain-text part gets the brand
/// name as a one-line header (a text part cannot carry an image). When the account HAS branding but
/// the message has no HTML part at all — the inline arms in [`get_inline`] return `text` only — an
/// HTML part is built from the text, escaped first, so the logo can actually render: the header is
/// the whole point of the feature and a client with no HTML part has nowhere to put it.
///
/// A no-op for an account with no branding, which is what makes this additive: those renders are
/// byte-identical to what they were before this feature existed.
fn with_branding_header(
    branding: Option<&crate::branding::Branding>,
    text: Option<String>,
    html: Option<String>,
) -> (Option<String>, Option<String>) {
    let Some(b) = branding else {
        return (text, html);
    };
    let logo = b.resolve_logo_url(APP_URL);
    let header = b.header_html(logo.as_deref());
    let html = match html {
        // Inside the document, never in front of it — see `branding::place_header` for the live
        // measurement. A template fragment is wrapped into a whole document so the header (and the
        // tenant's logo) cannot land outside the body a client may rebuild.
        Some(h) => Some(crate::branding::place_header(&h, &header)),
        None => text.as_deref().map(|t| {
            format!(
                "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head>\n<body style=\"margin:0;padding:0;\">{header}<div style=\"font:14px/1.6 -apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#111827;white-space:pre-wrap\">{}</div></body></html>",
                crate::branding::escape_html(t)
            )
        }),
    };
    let text = match (text, b.text_header()) {
        (Some(t), h) if !h.is_empty() => Some(format!("{h}{t}")),
        (t, _) => t,
    };
    (text, html)
}

/// Look up + render a template (DB-backed, falling back to inline hardcoded content) for ONE
/// account, with that account's email branding applied.
///
/// This is the single funnel every transactional send in this app passes through — `credentials`
/// (the new-account mail that carries the generated password), `password_reset`, and the two stored
/// types — which is why the branding is applied HERE rather than at each send site: a template added
/// later inherits it for free.
///
/// `brand_name` / `logo_url` join the merge map per render (they are per-account, so they cannot
/// live in [`bound_vars`], which is `&'static str`): the account's own values when it has branding,
/// the app's identity otherwise, so an admin-authored `{{brand_name}}` never goes out literally.
async fn render_template(
    pool: &PgPool,
    aid: Uuid,
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
) -> (String, Option<String>, Option<String>) {
    // A tenant with branding of its own wins; one without it gets the APP's header rather than no
    // header at all. `logo_is_own` is bound so the preview arm can say truthfully whose logo the
    // recipient is looking at instead of claiming an upload that never happened.
    let stored = crate::branding::load(pool, aid).await;
    let logo_is_own: &'static str = if stored
        .as_ref()
        .and_then(|b| b.resolve_logo_url(APP_URL))
        .is_some()
    {
        "yes"
    } else {
        "no"
    };
    let branding = stored.unwrap_or_else(app_default_branding);
    let brand_name = if branding.brand_name.is_empty() {
        APP_NAME.to_string()
    } else {
        branding.brand_name.clone()
    };
    let logo_url = branding.resolve_logo_url(APP_URL).unwrap_or_default();

    let mut all: std::collections::HashMap<&str, &str> =
        std::collections::HashMap::with_capacity(vars.len() + 2);
    for (k, v) in vars {
        all.insert(k, v);
    }
    all.insert("brand_name", &brand_name);
    all.insert("logo_url", &logo_url);
    all.insert("logo_is_own", logo_is_own);

    let (subject, body, html) = match get_db_template(pool, aid, template_type).await {
        Ok(Some((subject, body, html_body))) => (
            render(&subject, &all),
            body.map(|b| render(&b, &all)),
            html_body.map(|h| render(&h, &all)),
        ),
        _ => get_inline(template_type, &all),
    };

    let (body, html) = with_branding_header(Some(&branding), body, html);
    let (body, html) = with_support_footer(body, html);
    (subject, body, html)
}

/// Send a template email for a specific tenant, resolving the Mailgun/SMTP config from the
/// tenant's DB row, falling back to the global `admin_settings.email` row.
pub async fn send_template_email_for_tenant(
    pool: &PgPool,
    tenant_id: Uuid,
    aid: Uuid,
    to: &str,
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
) -> Result<(), String> {
    let (final_subject, final_body, final_html) =
        render_template(pool, aid, template_type, vars).await;

    dispatch_for_tenant(
        pool,
        Some(tenant_id),
        to,
        &final_subject,
        final_body.as_deref(),
        final_html.as_deref(),
    )
    .await
}

/// Fetch a template from the DB and send the email using the global system-mail provider
/// (no tenant context). Prefer `send_template_email_for_tenant` when a tenant_id is known.
pub async fn send_template_email(
    pool: &PgPool,
    aid: Uuid,
    to: &str,
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
) -> Result<(), String> {
    let (final_subject, final_body, final_html) =
        render_template(pool, aid, template_type, vars).await;

    dispatch_for_tenant(
        pool,
        None,
        to,
        &final_subject,
        final_body.as_deref(),
        final_html.as_deref(),
    )
    .await
}

/// Look up a template from `email_templates` — prefer account-specific, fall back to default
async fn get_db_template(
    pool: &PgPool,
    aid: Uuid,
    template_type: &str,
) -> Result<Option<(String, Option<String>, Option<String>)>, String> {
    let result = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"SELECT subject, body, html_body
           FROM email_templates
           WHERE template_type = $1 AND (aid = $2 OR is_default = true)
           ORDER BY is_default ASC, created_at DESC
           LIMIT 1"#,
    )
    .bind(template_type)
    .bind(aid)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query templates: {e}"))?;

    Ok(result)
}

/// Fallback hardcoded templates
fn get_inline(
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
) -> (String, Option<String>, Option<String>) {
    let name = vars.get("name").unwrap_or(&"there");
    let email = vars.get("email").unwrap_or(&"");
    let token = vars.get("token").unwrap_or(&"");
    let plan_name = vars.get("plan_name").unwrap_or(&"a plan");
    let app_url = vars
        .get("app_url")
        .unwrap_or(&"https://app.funnelswift.net");

    match template_type {
        "welcome" => {
            let subject = "Welcome to FunnelSwift!".to_string();
            let text = format!(
                "Welcome to FunnelSwift, {0}!\n\nYour account has been created successfully.\n\nEmail: {1}\n\nLogin at: {2}/login\n\nNext steps:\n- Create your first funnel\n- Set up your pages\n- Connect your domain\n- Launch your campaign\n\nBest regards,\nThe FunnelSwift Team",
                name, email, app_url
            );
            (subject, Some(text), None)
        }
        "purchase_confirmed" => {
            let subject = "Payment Received — Thank You!".to_string();
            let text = format!(
                "Hi {0},\n\nYour payment for {1} has been confirmed. Thank you!\n\nYou can access your account at {2}/login.\n\nThank you for your business!\n- FunnelSwift Team",
                name, plan_name, app_url
            );
            (subject, Some(text), None)
        }
        "password_reset" => {
            let subject = "Password Reset Request".to_string();
            // The customer must be TOLD where to enter the code — and in this app the ONLY route to
            // the code field is the login page's "Forgot password?" -> "Send reset code" button,
            // which marks the code in the mail as `used` (src/auth/handlers.rs, forgot_password).
            // The link therefore carries the code to the view that accepts it, so no second code is
            // ever needed (fixed 2026-10-13, cron `default`; link made into a one-click deep link
            // 2026-10-16, cron `default`, mirroring IncentiveSwift/WorkflowSwift).
            let text = format!(
                "Hi {0},\n\nWe received a request to reset the password on your FunnelSwift account.\n\nYour reset code is:\n\n{1}\n\nOpen {2}/login?token={1} to choose a new password - the code is filled in for you. If that link does not open the form, open {2}/login and paste the code above.\n\nThis code expires in 1 hour, so use it soon.\n\nIf you didn't ask for a password reset, you don't need to do anything - your password stays the same and no changes have been made to your account.\n\n- The FunnelSwift Team",
                name, token, app_url
            );
            (subject, Some(text), None)
        }
        // Change of sign-in address (cron 2026-10-18). The link carries the one-shot token to the
        // page that completes the change; the body names the new address so a customer who did not
        // ask for this can see the mistake immediately and ignore it (nothing changes until the link
        // is opened). A DB row named `email_change` overrides this arm; the wording here is what an
        // install without the row sends.
        "email_change" => {
            let new_email = vars.get("new_email").unwrap_or(&"").to_string();
            let link = vars.get("link").unwrap_or(&"").to_string();
            let subject = format!("Confirm your new {APP_NAME} sign-in address");
            let text = format!(
                "Hi {0},\n\nWe received a request to change the sign-in address on your {1} account to:\n\n{2}\n\nOpen this link to confirm the change:\n\n{3}\n\nYour sign-in address stays exactly as it is until you open that link. The link expires in 2 hours.\n\nIf you did not ask for this, ignore this message and nothing will change.\n\n- The {1} Team",
                name, APP_NAME, new_email, link
            );
            (subject, Some(text), None)
        }
        // David's model (2026-09-29): the signup form collects NAME + EMAIL only, the system
        // generates the first password and sends it here, and the user replaces it in their profile.
        // Before this arm existed there was NO email in this app that carried a password — the
        // generated password for an admin-created user was thrown away and the account was unusable.
        //
        // kanban t_36b55ed2 — ONE onboarding identity. The authoritative copy of this mail is the
        // `credentials` ROW in `email_templates` (admin-editable, seeded by migration); this arm is
        // the fallback for an install where that row is absent. It used to carry a DIFFERENT subject
        // ("Your FunnelSwift login details") and different wording, so the same signup produced two
        // different-looking onboarding mails depending on whether the row existed — and that, not a
        // double send, is the "two welcome emails" David saw (a row-less send at 16:13, the row's
        // copy at 16:26 once the migration landed). Subject and body below now mirror the row, so
        // the customer sees one message identity whichever path renders it.
        "credentials" => {
            let password = vars.get("password").unwrap_or(&"").to_string();
            let login_url = vars.get("login_url").unwrap_or(&APP_LOGIN_URL);
            let subject = format!("Your {APP_NAME} account is ready");
            let text = format!(
                "Hi {0},\n\nYour {1} account has been created and is ready to use.\n\nSign in at {2} with:\nEmail: {3}\nPassword: {4}\n\nThe password above is temporary. After you sign in, go to your profile settings and replace it with one of your own.\n\nIf you weren't expecting this email, someone may have entered your address by mistake - contact support and we'll take a look.\n\n- The {1} Team",
                name, APP_NAME, login_url, email, password
            );
            (subject, Some(text), None)
        }
        // A plan request is the ONLY way a customer can ask for a paid plan on this install (no
        // payment provider is configured, so checkout refuses). Until 2026-10-19 the operator's
        // answer was stored and the customer was never told: the ask sat in the row while the person
        // who wanted to pay heard nothing. This arm is the copy an install without a
        // `plan_request_update` template row sends; `reply` is empty when the operator wrote none.
        "plan_request_update" => {
            let plan = vars.get("plan_name").unwrap_or(&"a plan").to_string();
            let state_word = vars.get("status_word").unwrap_or(&"updated").to_string();
            let reply = vars.get("reply").unwrap_or(&"").to_string();
            let reply_line = if reply.is_empty() {
                String::new()
            } else {
                format!("A note from our team:\n\n{reply}\n\n")
            };
            let subject = format!("Your {APP_NAME} plan request - {plan}");
            let text = format!(
                "Hi {0},\n\nThanks for asking about the {1} plan. Your request is now {2}.\n\n{3}You can see the request and our reply any time on the Upgrade tab: {4}/upgrade\n\n- The {5} Team",
                name, plan, state_word, reply_line, app_url, APP_NAME
            );
            (subject, Some(text), None)
        }
        // The customer's RECEIPT (cron 2026-10-20). Filing a plan request is this install's only
        // buying path (checkout refuses: no payment provider is configured), and until now the
        // person who had just asked to pay heard nothing back — their screen showed a chip and
        // their inbox stayed empty. This is the acknowledgement: what was asked for, that a person
        // will answer by email, and that nothing has been charged. The operator's answer is the
        // separate `plan_request_update` arm below. `note` is empty when the customer wrote none.
        "plan_request_received" => {
            let plan = vars.get("plan_name").unwrap_or(&"a plan").to_string();
            let note = vars.get("note").unwrap_or(&"").to_string();
            let note_block = if note.is_empty() {
                String::new()
            } else {
                format!("What you told us:\n\n{note}\n\n")
            };
            let subject = format!("We have your {APP_NAME} plan request - {plan}");
            let text = format!(
                "Hi {0},\n\nThanks - your request for the {1} plan is with our team. You asked for the upgrade and nothing has been charged.\n\n{2}A person will reply to this address, and the answer appears on your Upgrade tab: {3}/upgrade\n\nIf you change your mind, you can withdraw the request from that same screen until we answer it.\n\n- The {4} Team",
                name, plan, note_block, app_url, APP_NAME
            );
            (subject, Some(text), None)
        }
        // The operator's own in-inbox proof of the branding above (cron 2026-10-23, parity with
        // IncentiveSwift). `render_template` has ALREADY swapped APP_NAME for the tenant's brand
        // name and `logo_url` for the tenant's logo, and `with_branding_header` has already put
        // that logo at the top of the HTML part — so this arm only has to say what was applied.
        // The recipient is always the CALLER's own address (the route takes no body), which is
        // what makes it a preview and not a relay. A template row named `branding_preview`, if an
        // install has one, wins; this is the fallback for an install that has none.
        "branding_preview" => {
            let brand = vars.get("brand_name").unwrap_or(&APP_NAME).to_string();
            // `logo_url` is ALWAYS set now (the app's own wordmark is the fallback), so this reads
            // the owner flag instead: an upload the tenant never made must not be described as one.
            let has_logo = vars.get("logo_is_own").copied() == Some("yes");
            let subject = format!("A preview of your {brand} email");
            let logo_line = if has_logo {
                "The logo you uploaded sits at the top of the message above this text.\n\n"
            } else {
                "No logo of your own is set yet, so this message carries the FunnelSwift wordmark. Upload a logo on the dashboard's Email branding card, save it, and send yourself another preview to see your own.\n\n"
            };
            let text = format!(
                "Hi {0},\n\nThis is what the mail your customers receive looks like now.\n\n{1}The brand name on it is \"{2}\", and every message this app sends is signed the same way.\n\nNothing else about your account changed, and there is nothing to do — this message exists so you can see the result in a real inbox instead of taking it on trust.\n\n- The {2} Team",
                name, logo_line, brand
            );
            (subject, Some(text), None)
        }
        // A NEW LEAD ON A PUBLIC FUNNEL (cron 2026-11-01). Capturing a lead is this app's whole
        // job, and until now a captured lead landed in a table the owner had to remember to open:
        // a visitor filled in the form, the row was written, and nobody was told. The recipient is
        // the workspace owner's OWN address (never the visitor's), so the message can be answered
        // by replying to it. A template row named `new_lead` overrides this arm; `lead_phone` is
        // empty for a funnel that does not ask for a number.
        "new_lead" => {
            let funnel = vars
                .get("funnel_name")
                .unwrap_or(&"your funnel")
                .to_string();
            let lead_name = vars.get("lead_name").unwrap_or(&"A visitor").to_string();
            let lead_email = vars.get("lead_email").unwrap_or(&"").to_string();
            let lead_phone = vars.get("lead_phone").unwrap_or(&"").to_string();
            let phone_line = if lead_phone.is_empty() {
                String::new()
            } else {
                format!("Phone: {lead_phone}\n")
            };
            let subject = format!("New lead on {funnel}");
            let text = format!(
                "Hi {0},\n\nSomeone just filled in {1}.\n\nName: {2}\nEmail: {3}\n{4}\nEvery lead is stored on your dashboard: {5}/dashboard\n\nReply to this message to answer them directly.\n\n- The {6} Team",
                name, funnel, lead_name, lead_email, phone_line, app_url, APP_NAME
            );
            (subject, Some(text), None)
        }
        // A SUPPORT REQUEST THE OPERATOR HAS ANSWERED (cron 2026-11-06). A customer files a
        // request from the Support screen and reads the answer there — but only if they happen to
        // go back and look, and nothing told them there was anything to look at. The plan-request
        // answer beside this arm has mailed the person who asked since 2026-10-19 for exactly this
        // reason; the support request, built on 2026-11-06, stored the sentence and told nobody.
        // `subject_line` is the customer's own words so the mail is recognisable in a list, and
        // `reply` is the operator's answer quoted whole. A template row named `support_reply`
        // overrides this arm.
        "support_reply" => {
            // The brand the workspace signed up under, not this crate's constant: a tenant with
            // its own branding must be answered by its own name, exactly like the branding preview
            // arm reads the bound value rather than APP_NAME.
            let brand = vars.get("brand_name").unwrap_or(&APP_NAME).to_string();
            let asked = vars
                .get("subject_line")
                .unwrap_or(&"your request")
                .to_string();
            let reply = vars.get("reply").unwrap_or(&"").to_string();
            let status_word = vars.get("status_word").unwrap_or(&"answered").to_string();
            let subject = format!("Your {brand} support request - {asked}");
            // WHERE THE ANSWER LIVES DEPENDS ON WHO ASKED (2026-11-06). This arm used to send
            // EVERY reader to the account's Support screen - but a message filed from
            // /contact.html belongs to no workspace, so that reader has no account to sign in to
            // and no Support screen to open. The sentence pointed them at a door they cannot
            // unlock, and the reference they were handed at filing time appeared nowhere in the
            // mail, so the one route that WOULD work was never named. Branch on it.
            let has_account = vars
                .get("has_account")
                .map(|v| v.to_string())
                .unwrap_or_default()
                == "true";
            let read_back = if has_account {
                format!(
                    "You can read it any time on the Support screen in your account - sign in at {app_url} and open Support in the left-hand menu."
                )
            } else {
                let reference = vars
                    .get("reference")
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                format!(
                    "You do not need an account to read it again: open {app_url}/contact.html and enter your reference {reference} together with the address you wrote from."
                )
            };
            let text = format!(
                "Hi {0},\n\nYou asked us:\n\n{1}\n\nOur answer, from the {2} team:\n\n{3}\n\nYour request now reads {4}. {5}\n\nReplying to this message reaches the same team, so you never have to file a second request to answer us back.\n\n- The {2} Team",
                name, asked, brand, reply, status_word, read_back
            );
            (subject, Some(text), None)
        }
        "lead_cap_notice" => {
            // Plain language on purpose: this mail is read by a business owner who did not change
            // anything and whose forms have quietly stopped collecting. It says what happened,
            // that nothing is broken, and the one action that fixes it.
            let noun = vars.get("noun").unwrap_or(&"entry");
            let text = format!(
                "Hi {0},\n\nA form on your account just tried to take a new {1} and was stopped, because your workspace has reached the {1} allowance on {2}.\n\nWhat that means: visitors filling in your public form are now being told to contact you directly, so that {1} was not captured. Nothing is broken - the plan limit is doing exactly what it says.\n\nThe fix is one step. Open your account at {3}, go to Plans, and choose a plan with a bigger {1} allowance. Your forms start collecting again as soon as the new plan is active.\n\nIf you would rather stay on {2}, check whether every {1} you are holding is still one you want - the allowance only frees up when you delete one.\n\n- The FunnelSwift Team",
                name, noun, plan_name, app_url
            );
            (CEILING_SUBJECT.to_string(), Some(text), None)
        }
        _ => {
            let subject = "FunnelSwift Notification".to_string();
            (subject, Some(format!("{}", json!(vars))), None)
        }
    }
}

// Convenience wrappers for backwards compatibility.
// Each one binds its own merge fields and then `bound_vars` adds the shared
// `app_name` / `app_url` / `login_url` fields the admin UI advertises.
pub async fn send_welcome_email(
    pool: &PgPool,
    aid: Uuid,
    to: &str,
    name: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("email", to);
    send_template_email(pool, aid, to, "welcome", &bound_vars(vars)).await
}

/// Send the generated first password. David: *"the email should automatically go out with generated
/// login credentials ... instead of the form saying create their own password."*
pub async fn send_credentials_email(
    pool: &PgPool,
    aid: Uuid,
    to: &str,
    name: &str,
    password: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("email", to);
    vars.insert("password", password);
    send_template_email(pool, aid, to, "credentials", &bound_vars(vars)).await
}

pub async fn send_purchase_confirmed_email(
    pool: &PgPool,
    aid: Uuid,
    to: &str,
    name: &str,
    plan_name: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("plan_name", plan_name);
    send_template_email(pool, aid, to, "purchase_confirmed", &bound_vars(vars)).await
}

/// The operator's own in-inbox proof that their branding is applied (cron 2026-10-23, parity with
/// IncentiveSwift). Recipient is always the caller's own address — this is a preview, not a relay.
pub async fn send_branding_preview_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    name: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("email", to);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "branding_preview",
        &bound_vars(vars),
    )
    .await
}

/// One rendered `branding_preview` sample, ready to judge — never delivered.
pub struct RenderedPreview {
    pub subject: String,
    pub html: String,
    pub text: String,
    /// True when the header above the body is THIS account's own logo, not the app's wordmark.
    pub own_logo: bool,
    /// True when the account has saved branding at all (a name or a logo).
    pub branded: bool,
}

/// The EXACT parts a `branding_preview` sample would carry, rendered but NOT sent.
///
/// WHY (cron 2026-10-28, parity with MissedCall Respondr): the console's "How it looks in a mail"
/// block draws its OWN mirror of [`crate::branding::Branding::header_html`], so it can drift from
/// the renderer — an operator then approves a header that is not the one their customers receive.
/// It also meant the fleet could never prove an UPLOADED logo reaches the outgoing body without
/// mailing a real inbox. This answers the panel with the bytes the mail path would emit — same merge
/// fields, same template lookup, same header, same support footer, in the same order as
/// [`send_template_email_for_tenant`] — and delivers nothing.
pub async fn render_branding_preview(
    pool: &PgPool,
    aid: Uuid,
    name: &str,
    to: &str,
) -> RenderedPreview {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("email", to);
    let vars = bound_vars(vars);
    let (subject, body, html) = render_template(pool, aid, "branding_preview", &vars).await;
    // Read AFTER the render so the flags describe the same branding rows the render used.
    let stored = crate::branding::load(pool, aid).await;
    let own_logo = stored
        .as_ref()
        .and_then(|b| b.resolve_logo_url(APP_URL))
        .is_some();
    RenderedPreview {
        subject,
        html: html.unwrap_or_default(),
        text: body.unwrap_or_default(),
        own_logo,
        branded: stored.is_some(),
    }
}

pub async fn send_reset_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    token: &str,
    name: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("token", token);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "password_reset",
        &bound_vars(vars),
    )
    .await
}

/// Acknowledge a plan request the moment the customer files it (cron 2026-10-20).
///
/// Called from `POST /api/v1/plan-requests`, after the row is written. The row is the record the
/// operator works from; this mail is what tells the person who just asked to pay that the ask
/// arrived. Best effort by design — a mail failure must never lose the request, so the caller logs
/// and keeps the `201`. Renders through the tenant's own templates and branding, like every other
/// mail this app sends, and a reserved/harness recipient is withheld and recorded as a non-send.
#[allow(clippy::too_many_arguments)]
pub async fn send_plan_request_received_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    name: &str,
    plan_name: &str,
    note: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("plan_name", plan_name);
    vars.insert("note", note);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "plan_request_received",
        &bound_vars(vars),
    )
    .await
}

/// Tell the OPERATOR that a workspace asked to buy a plan (2026-11-09).
///
/// `POST /api/v1/plan-requests` already mails the person who filed the ask; this is the other half
/// of the same event, and it was missing: nothing told the person who can actually move the row. On
/// an installation with no payment provider configured the request IS the buying path, so an ask
/// nobody is told about is a sale nobody makes.
///
/// The destination is the operator's own address, stored in the platform settings
/// (`operator_alert_email`, set from Admin → System Settings). An unset address sends nothing —
/// this code never invents a mailbox. Deliberately sent through the SYSTEM mail configuration
/// (`None`), not the asking workspace's, because this message is platform mail: it must arrive even
/// when the customer's own mail server is the thing that is broken.
///
/// Best effort by design: a mail failure must never lose the request — the row is already written
/// and the console's queue reads it — so the caller logs and keeps its 201.
pub async fn send_plan_request_operator_alert(
    pool: &PgPool,
    tenant_id: Uuid,
    plan_name: &str,
    requester_email: &str,
    note: &str,
) -> Result<(), String> {
    let settings = crate::platform_settings::load(pool).await;
    let Some(to) = settings
        .operator_alert_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    // The workspace name is what the operator recognises; a missing/blank name falls back to the
    // address of the person who asked, never to a made-up label.
    let workspace: String = sqlx::query_scalar::<_, Option<String>>(
        "SELECT NULLIF(btrim(name), '') FROM tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("tenant name lookup: {e}"))?
    .flatten()
    .unwrap_or_else(|| requester_email.to_string());
    // A FLEET PROBE MUST NEVER RING THE OPERATOR (kanban t_f17b5802): the recipient here is the
    // operator whatever address the requester gave, so a walk that files a request pages a human.
    // The predicate is on the REQUESTER: reserved domain, harness local part, or a
    // harness-shaped workspace name. The request row still lands in the console queue.
    if let Some(reason) =
        crate::security::probe_addr::synthetic_requester(requester_email, &workspace)
    {
        tracing::info!(
            reason = %reason,
            "plan-request operator alert WITHHELD — the requester is a synthetic identity"
        );
        return Ok(());
    }
    let note = if note.trim().is_empty() {
        "(none)"
    } else {
        note.trim()
    };
    let subject = format!("A customer asked to buy {plan_name}");
    let text = format!(
        "{workspace} asked to be moved onto {plan_name}.\n\n\
         Requested by: {requester_email}\n\
         Plan: {plan_name}\n\
         Note from the customer: {note}\n\n\
         The request is waiting in the admin console (Plans -> Plan requests). Moving it to done \
         puts that workspace on the plan."
    );
    let html = format!(
        "<p><strong>{}</strong> asked to be moved onto <strong>{}</strong>.</p>\
         <p>Requested by: {}</p>\
         <p>Note from the customer: {}</p>\
         <p>The request is waiting in the admin console (Plans &rarr; Plan requests). Moving it to \
         <em>done</em> puts that workspace on the plan.</p>",
        crate::templates::html_escape(&workspace),
        crate::templates::html_escape(plan_name),
        crate::templates::html_escape(requester_email),
        crate::templates::html_escape(note),
    );
    dispatch_for_tenant(pool, None, to, &subject, Some(&text), Some(&html)).await
}

/// Tell the OPERATOR that a new workspace just signed up (2026-11-09).
///
/// WHY THIS EXISTS (measured 2026-11-09): both signup doors mint a tenant, a user, a plan
/// subscription and mail the customer their first password — and NOTHING told the person who runs
/// the install. The plan-request and support-request arms next door already mail
/// `operator_alert_email`; a new customer is the same kind of news (somebody is now using the
/// product), and the operator's own dashboard is the only place it appears, so the arrival of a
/// customer is invisible until somebody happens to open the console.
///
/// Recipient = the operator's own `operator_alert_email` (Admin -> System Settings). An unset
/// address sends nothing; this code never invents a mailbox. Sent through the SYSTEM mail
/// configuration (`None`), never the new workspace's, because the workspace has no sender identity
/// yet and the message is platform mail.
///
/// Never called for a probe/harness signup: the caller gates on the same two suppression reasons
/// the credentials mail uses, so machinery cannot mail the operator.
///
/// Best effort by design: the account is already committed, and a mail failure must never turn a
/// successful signup into an error. The caller logs and keeps its 201.
pub async fn send_signup_operator_alert(
    pool: &PgPool,
    tenant_id: Uuid,
    owner_name: &str,
    owner_email: &str,
    plan_name: &str,
    source: &str,
) -> Result<(), String> {
    let settings = crate::platform_settings::load(pool).await;
    let Some(to) = settings
        .operator_alert_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    // The workspace name is what the operator recognises; a blank one falls back to the owner's
    // address, never to an invented label.
    let workspace: String = sqlx::query_scalar::<_, Option<String>>(
        "SELECT NULLIF(btrim(name), '') FROM tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("tenant name lookup: {e}"))?
    .flatten()
    .unwrap_or_else(|| owner_email.to_string());
    let who = if owner_name.trim().is_empty() {
        owner_email.to_string()
    } else {
        format!("{} ({})", owner_name.trim(), owner_email)
    };
    let source = if source.trim().is_empty() {
        "unknown"
    } else {
        source.trim()
    };
    let plan_name = if plan_name.trim().is_empty() {
        "no plan attached"
    } else {
        plan_name.trim()
    };
    let subject = format!("New workspace: {workspace}");
    let text = format!(
        "A new workspace signed up.\n\n\
         Workspace: {workspace}\n\
         Owner: {who}\n\
         Plan: {plan_name}\n\
         Source: {source}\n\n\
         Their login credentials were emailed to the owner. The workspace is in the admin console \
         (Tenants) and on the dashboard."
    );
    let html = format!(
        "<p>A new workspace signed up.</p>\
         <p><strong>{}</strong><br>\
         Owner: {}<br>\
         Plan: {}<br>\
         Source: {}</p>\
         <p>Their login credentials were emailed to the owner. The workspace is in the admin console \
         (Tenants) and on the dashboard.</p>",
        crate::templates::html_escape(&workspace),
        crate::templates::html_escape(&who),
        crate::templates::html_escape(plan_name),
        crate::templates::html_escape(source),
    );
    dispatch_for_tenant(pool, None, to, &subject, Some(&text), Some(&html)).await
}

/// Tell the OPERATOR that a customer wrote to support (2026-11-09).
///
/// WHY THIS EXISTS (measured live 2026-11-09): `POST /api/v1/support-requests` and its public twin
/// both write the row and both return 201, and NOTHING told the person who answers them — no mail,
/// no alert, no cron. The queue in the admin console was the only place the message existed, so a
/// support request sat unseen until somebody happened to open the screen. The plan-request arm next
/// door already mails `operator_alert_email` for exactly this reason; a help request is the same
/// promise with a higher cost when it is missed.
///
/// Recipient = the operator's own `operator_alert_email` (Admin -> System Settings). An unset
/// address sends nothing: this code never invents a mailbox, and the address is the operator's to
/// choose. Sent through the SYSTEM mail configuration (`None`), never the asking workspace's, so it
/// still arrives when the customer's own mail server is the broken thing.
///
/// Best effort by design: the row is already written and the console's queue reads it, so a mail
/// failure must never turn a filed request into a 500. The caller logs and keeps its 201.
pub async fn send_support_request_operator_alert(
    pool: &PgPool,
    tenant_id: Option<Uuid>,
    from_name: &str,
    from_email: &str,
    subject_line: &str,
    message: &str,
) -> Result<(), String> {
    let settings = crate::platform_settings::load(pool).await;
    let Some(to) = settings
        .operator_alert_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    // Who wrote in: the workspace's name when the request carries one, the writer's own name when
    // they gave it, and the address as the last honest fallback.
    let who = match tenant_id {
        Some(tid) => {
            let tenant: Option<String> = sqlx::query_scalar::<_, Option<String>>(
                "SELECT NULLIF(btrim(name), '') FROM tenants WHERE id = $1",
            )
            .bind(tid)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("tenant name lookup: {e}"))?
            .flatten();
            tenant.unwrap_or_else(|| from_email.to_string())
        }
        None => {
            if from_name.trim().is_empty() {
                format!("A visitor (not signed in): {from_email}")
            } else {
                format!("{} (not signed in): {}", from_name.trim(), from_email)
            }
        }
    };
    // A FLEET PROBE MUST NEVER RING THE OPERATOR (kanban t_f17b5802). Requester, not route.
    if let Some(reason) = crate::security::probe_addr::synthetic_requester(from_email, &who) {
        tracing::info!(
            reason = %reason,
            "support operator alert WITHHELD — the filer is a synthetic identity"
        );
        return Ok(());
    }
    let subject = format!("A support request needs an answer: {subject_line}");
    let text = format!(
        "{who} wrote to support.\n\n\
         Subject: {subject_line}\n\
         Reply to: {from_email}\n\n\
         {message}\n\n\
         Answer it in the admin console (Support -> Support requests). The customer sees the answer \
         on their own Support screen and is mailed a copy."
    );
    let html = format!(
        "<p><strong>{}</strong> wrote to support.</p>\
         <p>Subject: {}</p>\
         <p>Reply to: {}</p>\
         <pre style=\"white-space:pre-wrap;font-family:inherit\">{}</pre>\
         <p>Answer it in the admin console (Support &rarr; Support requests). The customer sees the \
         answer on their own Support screen and is mailed a copy.</p>",
        crate::templates::html_escape(&who),
        crate::templates::html_escape(subject_line),
        crate::templates::html_escape(from_email),
        crate::templates::html_escape(message),
    );
    dispatch_for_tenant(pool, None, to, &subject, Some(&text), Some(&html)).await
}

/// Tell a workspace owner that a visitor just filled in one of their public funnels (2026-11-01).
///
/// Called from `POST /funnel/:slug/submit`, after the `leads` row and its `lead.created` webhook
/// are written. The recipient is the OWNER's own address — never the visitor's — so pressing reply
/// answers the person who filled the form. Best effort by design: a mail failure must never lose
/// the capture, so the caller logs it and keeps the `201`. Renders through the tenant's own
/// templates and branding like every other mail here, and a reserved/harness recipient is withheld
/// and recorded as a non-send by `dispatch_for_tenant`.
#[allow(clippy::too_many_arguments)]
pub async fn send_new_lead_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    owner_name: &str,
    lead_name: &str,
    lead_email: &str,
    lead_phone: Option<&str>,
    funnel_name: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", owner_name);
    vars.insert("lead_name", lead_name);
    vars.insert("lead_email", lead_email);
    vars.insert("lead_phone", lead_phone.unwrap_or(""));
    vars.insert("funnel_name", funnel_name);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "new_lead",
        &bound_vars(vars),
    )
    .await
}

/// The exact subject of the plan-ceiling notice.
///
/// It doubles as the once-a-day dedupe key in `email_send_log` (that table stores the subject, not
/// a template name), so it must stay a CONSTANT: a tenant name, a date or a count in here would
/// defeat the dedupe and turn a holding ceiling into a mail flood.
const CEILING_SUBJECT: &str = "Your lead capture has stopped at your plan's limit";

/// Tell a workspace's OWNER that the plan's ceiling has stopped their public capture.
///
/// Called from [`crate::features::enforce_public_lead_limit`] — the gate a stranger's form submit
/// hits. This is a NOTICE, not a refusal: the lead is already lost, so the mail exists only to make
/// sure the one person who can lift the ceiling finds out the day it happens, instead of finding
/// out when the next campaign under-performs and nobody can say why. Until this function existed
/// the ceiling was silent to them: the prospect got a neutral sentence and the app log got a
/// warning, and neither is read by the customer.
///
/// Silent for 24 hours after each notice, and silent for a workspace with no active user. Every
/// failure is returned to the caller, which only logs it — the refusal itself is never affected.
pub async fn notify_owner_of_public_ceiling(
    pool: &PgPool,
    tenant_id: Uuid,
    feature_key: &str,
) -> Result<(), String> {
    // The account holder is the one who can buy the fix. `users.role` is free text on this app
    // (measured: company_admin / admin / user), so order with a CASE rather than binding a word
    // that may not be in the vocabulary.
    let owner: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT email, name FROM users \
          WHERE tenant_id = $1 AND is_active = true \
          ORDER BY CASE role WHEN 'company_admin' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, created_at \
          LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some((email, name)) = owner else {
        return Ok(());
    };

    // Once a day per address. Read from the delivery history so a WITHHELD notice (a probe
    // workspace) also counts — otherwise a harness tenant at its ceiling would retry forever. The
    // match is a PREFIX because a withheld row is recorded as "<subject> (withheld)".
    let already: Option<(i64,)> = sqlx::query_as(
        "SELECT 1::bigint FROM email_send_log \
          WHERE lower(to_email) = lower($1) AND subject LIKE $2 || '%' \
            AND created_at > NOW() - INTERVAL '24 hours' \
          LIMIT 1",
    )
    .bind(&email)
    .bind(CEILING_SUBJECT)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if already.is_some() {
        return Ok(());
    }

    let plan_name: String = sqlx::query_scalar(
        "SELECT p.name FROM plans p \
           JOIN tenant_plan_subscriptions s ON s.plan_id = p.id \
          WHERE s.tenant_id = $1 AND s.status = 'active' \
          ORDER BY s.start_date DESC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or_else(|| "your current plan".to_string());

    let noun = match feature_key {
        "max_leads" => "lead",
        "max_cards" => "card",
        "max_funnels" => "funnel",
        "max_users" => "team member",
        _ => "entry",
    };

    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name.as_deref().unwrap_or("there"));
    vars.insert("plan_name", plan_name.as_str());
    vars.insert("noun", noun);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        &email,
        "lead_cap_notice",
        &bound_vars(vars),
    )
    .await
}

/// Tell the customer their plan request moved on.
///
/// Called from `PUT /api/v1/admin/plan-requests/:id`. The operator's answer is only worth writing if
/// the person who asked hears it — and while no payment provider is configured this request IS the
/// sales path, so an answer nobody reads is a lost sale. Renders through the tenant's own templates
/// and branding, like every other mail this app sends.
pub async fn send_plan_request_update_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    name: &str,
    plan_name: &str,
    status_word: &str,
    reply: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("plan_name", plan_name);
    vars.insert("status_word", status_word);
    vars.insert("reply", reply);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "plan_request_update",
        &bound_vars(vars),
    )
    .await
}

/// Tell the person who ASKED that the operator has answered their support request.
///
/// The row is the record and stays the record; this only carries the sentence to the inbox of the
/// person who filed it, because an answer nobody is told about is an answer nobody reads. The
/// caller records whether the attempt succeeded and NEVER fails the write on it: a dead mail
/// provider must not lose the operator's reply. A reserved/harness recipient is withheld by the
/// send funnel (no provider request at all) and lands in `email_send_log` as `withheld`, so a
/// probe answer is neither delivered nor misreported as delivered.
/// ONE notice, in one value: `clippy::too_many_arguments` is law here, and the seven plain
/// parameters were already six before the reference was added. `tenant_id = None` is a message
/// filed from the CONTACT page — that reader has no workspace, so the copy must send them to the
/// public read-back instead of a Support screen they cannot open.
pub struct SupportReplyNotice<'a> {
    pub tenant_id: Option<Uuid>,
    pub to: &'a str,
    pub name: &'a str,
    pub subject_line: &'a str,
    pub status_word: &'a str,
    pub reply: &'a str,
    /// The support request's own id — the reference the filing page hands the visitor and the
    /// credential `POST /api/v1/public/support-answer` reads the answer back with.
    pub reference: &'a str,
}

pub async fn send_support_reply_email(
    pool: &PgPool,
    notice: SupportReplyNotice<'_>,
) -> Result<(), String> {
    let SupportReplyNotice {
        tenant_id,
        to,
        name,
        subject_line,
        status_word,
        reply,
        reference,
    } = notice;
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("subject_line", subject_line);
    vars.insert("status_word", status_word);
    vars.insert("reply", reply);
    vars.insert("reference", reference);
    // `tenant_id.is_none()` is the contact-page visitor: no workspace, therefore no Support
    // screen and no account to sign in to. The mail must say so.
    vars.insert(
        "has_account",
        if tenant_id.is_some() { "true" } else { "false" },
    );
    send_template_email_for_tenant(
        pool,
        tenant_id.unwrap_or_default(),
        tenant_id.unwrap_or_default(),
        to,
        "support_reply",
        &bound_vars(vars),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &[(&'a str, &'a str)]) -> std::collections::HashMap<&'a str, &'a str> {
        pairs.iter().copied().collect()
    }

    // The plan-request answer is the one mail this app sends on the operator's behalf rather than
    // on a customer's action, and it is composed from an inline arm (no template row on a fresh
    // install). A withheld row stores only the KIND, never the subject, so the copy is asserted
    // here — the live probe can prove the recipient and the count, not the words.
    #[test]
    fn plan_request_update_arm_names_the_plan_and_carries_the_note() {
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("plan_name", "Kinetic Pro"),
            ("status_word", "complete"),
            ("reply", "Your workspace is live on Kinetic Pro."),
        ]));
        let (subject, text, _html) = get_inline("plan_request_update", &v);
        let text = text.unwrap_or_default();
        assert!(subject.contains("Kinetic Pro"), "subject was {subject}");
        assert!(text.contains("Your workspace is live on Kinetic Pro."));
        assert!(text.contains("A note from our team"));
        assert!(
            text.contains("/upgrade"),
            "the customer must be told where to look"
        );
    }

    #[test]
    fn support_reply_tells_a_contact_page_reader_where_the_answer_actually_is() {
        // BOTH halves, because a one-armed assertion cannot tell a real branch from a hard-coded
        // swap: a contact-page visitor has no workspace and no Support screen, and a workspace
        // customer must not be sent to the public form instead of their own Support screen.
        let guest = bound_vars(vars(&[
            ("name", "Probe"),
            ("subject_line", "Cannot sign in"),
            ("status_word", "answered"),
            ("reply", "Your account is unlocked."),
            ("reference", "ref-9f2a7c1e"),
            ("has_account", "false"),
        ]));
        let (subject, text, _html) = get_inline("support_reply", &guest);
        let text = text.unwrap_or_default();
        assert!(subject.contains("Cannot sign in"), "subject was {subject}");
        assert!(
            text.contains("ref-9f2a7c1e"),
            "the reference the reader needs was not quoted: {text}"
        );
        assert!(
            text.contains("/contact.html"),
            "a reader with no account was not told the page that works: {text}"
        );
        assert!(
            !text.contains("Support screen in your account"),
            "a reader with no account was sent to a screen they cannot open: {text}"
        );

        let customer = bound_vars(vars(&[
            ("name", "Customer"),
            ("subject_line", "Invoice"),
            ("status_word", "answered"),
            ("reply", "Refunded."),
            ("reference", "ref-9f2a7c1e"),
            ("has_account", "true"),
        ]));
        let (_subject, text, _html) = get_inline("support_reply", &customer);
        let text = text.unwrap_or_default();
        assert!(
            text.contains("Support screen in your account"),
            "a workspace customer lost their route to the answer: {text}"
        );
        assert!(
            !text.contains("/contact.html"),
            "a workspace customer was sent to the public form instead: {text}"
        );
    }

    #[test]
    fn plan_request_update_arm_omits_the_note_block_when_there_is_no_note() {
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("plan_name", "Kinetic Pro"),
            ("status_word", "received"),
            ("reply", ""),
        ]));
        let (_subject, text, _html) = get_inline("plan_request_update", &v);
        let text = text.unwrap_or_default();
        assert!(
            !text.contains("A note from our team"),
            "empty note block leaked"
        );
        assert!(text.contains("Kinetic Pro"));
    }

    // The support answer is the second mail this app sends on the operator's behalf rather than on
    // a customer's action, and it is composed from an inline arm (no template row on a fresh
    // install). A withheld row stores only the KIND, never the subject, so the copy is asserted
    // here — the live probe can prove the recipient and the count, not the words.
    #[test]
    fn support_reply_arm_carries_the_answer_and_says_where_to_read_it() {
        // A WORKSPACE customer: this arm keeps the Support-screen route it has always named. The
        // contact-page reader's arm (no account, reference + /contact.html instead) is asserted
        // separately in `support_reply_tells_a_contact_page_reader_where_the_answer_actually_is`.
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("subject_line", "Checkout will not take my card"),
            ("status_word", "answered"),
            ("reply", "Stripe was in test mode; it is live again now."),
            ("reference", "ref-9f2a7c1e"),
            ("has_account", "true"),
        ]));
        let (subject, text, _html) = get_inline("support_reply", &v);
        let text = text.unwrap_or_default();
        assert!(
            subject.contains("Checkout will not take my card"),
            "subject was {subject}"
        );
        assert!(text.contains("Stripe was in test mode; it is live again now."));
        assert!(
            text.contains("Support"),
            "the customer must be told where to look"
        );
        assert!(text.contains("answered"), "subject was {subject}");
    }

    #[test]
    fn support_reply_arm_names_the_workspace_brand_when_one_is_applied() {
        let pairs = [
            ("name", "Probe"),
            ("subject_line", "Cannot sign in"),
            ("status_word", "answered"),
            ("reply", "Try the reset link again."),
        ];
        let mut v = bound_vars(vars(&pairs));
        v.insert("brand_name", "Probe Co");
        let (_subject, text, _html) = get_inline("support_reply", &v);
        let text = text.unwrap_or_default();
        // `render_template` swaps APP_NAME for the tenant's brand before this arm runs; the arm
        // must therefore read the bound value rather than the constant, or a branded workspace's
        // customer is answered by "FunnelSwift" instead of the name they signed up for.
        assert!(text.contains("Probe Co"), "text was {text}");
        assert!(!text.contains("FunnelSwift"), "text was {text}");
    }

    #[test]
    fn plan_request_received_arm_acknowledges_the_ask_and_names_no_charge() {
        // cron 2026-10-20 — the receipt a customer gets the moment they file the only buying path
        // this install offers. Asserts the three facts the copy exists to carry: which plan, that a
        // person will answer, and that nothing was charged.
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("plan_name", "Kinetic Pro"),
            ("note", "please call me on Monday"),
        ]));
        let (subject, text, _html) = get_inline("plan_request_received", &v);
        let text = text.unwrap_or_default();
        assert!(subject.contains("Kinetic Pro"), "subject was {subject}");
        assert!(text.contains("nothing has been charged"));
        assert!(text.contains("please call me on Monday"));
        assert!(text.contains("/upgrade"), "where to look is missing");
    }

    #[test]
    fn plan_request_received_arm_omits_the_note_block_when_there_is_no_note() {
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("plan_name", "Kinetic Pro"),
            ("note", ""),
        ]));
        let (_subject, text, _html) = get_inline("plan_request_received", &v);
        let text = text.unwrap_or_default();
        assert!(!text.contains("What you told us"), "empty note leaked");
        assert!(text.contains("Kinetic Pro"));
    }

    #[test]
    fn render_substitutes_double_braces() {
        let v = bound_vars(vars(&[("name", "PW Test")]));
        assert_eq!(render("Hi {{name}}", &v), "Hi PW Test");
    }

    #[test]
    fn render_leaves_single_braces_literal_and_reports_them() {
        // The live `welcome` row shipped as `{app_name}` / `{name}`: render() replaced
        // nothing, so the recipient saw the placeholder text verbatim. The contract is
        // double braces only — and the leftovers are now reportable, not silent.
        let v = bound_vars(vars(&[("name", "PW Test")]));
        let out = render("Hi {name}", &v);
        assert_eq!(out, "Hi {name}");
        assert_eq!(unsubstituted(&out), vec!["name"]);
    }

    #[test]
    fn unsubstituted_reports_each_missing_key_once() {
        assert_eq!(unsubstituted("{{a}} {b} {{a}}"), vec!["a", "b"]);
        assert!(unsubstituted("no placeholders here").is_empty());
        assert!(unsubstituted("").is_empty());
    }

    #[test]
    fn bound_vars_cover_every_advertised_merge_field() {
        // Exactly the lists served by GET /api/v1/admin/email-templates/types.
        // kanban t_2f9a6576: `credentials` (the signup mail that carries the generated password)
        // was absent from this list, so the panel could not offer the type and an admin could not
        // edit the one mail every new customer receives.
        let advertised: [&[&str]; 5] = [
            &[
                "name",
                "email",
                "password",
                "login_url",
                "app_name",
                "brand_name",
                "logo_url",
            ],
            &["name", "token", "app_name", "brand_name", "logo_url"],
            &[
                "name",
                "email",
                "login_url",
                "app_name",
                "brand_name",
                "logo_url",
            ],
            &[
                "name",
                "plan_name",
                "login_url",
                "app_name",
                "brand_name",
                "logo_url",
            ],
            &[
                "name",
                "subject_line",
                "status_word",
                "reply",
                "app_name",
                "brand_name",
                "logo_url",
            ],
        ];
        let bound = bound_vars(vars(&[
            ("name", "n"),
            ("email", "e"),
            ("password", "pw"),
            ("token", "t"),
            ("plan_name", "pl"),
            ("subject_line", "s"),
            ("status_word", "w"),
            ("reply", "r"),
        ]));
        for list in advertised {
            for field in list {
                assert!(
                    bound.contains_key(*field),
                    "advertised merge field {field} is bound by no sender"
                );
            }
        }
    }

    #[test]
    fn credentials_path_renders_the_generated_password() {
        // GAP 1 (kanban t_2f9a6576): the new-account mail must show the generated first password —
        // it is the customer's ONLY way in, because the signup form never asks for one. The render
        // map substitutes any bound key, so the contract to hold is that this sender binds
        // `password` and that every advertised field renders with no placeholder left over.
        let advertised = ["name", "email", "password", "login_url", "app_name"];
        let v = bound_vars(vars(&[
            ("name", "Ada Lovelace"),
            ("email", "ada@example.com"),
            ("password", "S3cret-pw!"),
        ]));
        let tpl = "Hi {{name}},\n\nEmail: {{email}}\nPassword: {{password}}\nSign in: {{login_url}}\n\n- The {{app_name}} Team";
        let out = render(tpl, &v);
        assert!(
            unsubstituted(&out).is_empty(),
            "placeholder left on the wire: {:?}",
            unsubstituted(&out)
        );
        assert!(
            out.contains("S3cret-pw!"),
            "the generated password is not in the body"
        );
        assert!(!out.contains('{'));
        for field in advertised {
            assert!(
                v.contains_key(field),
                "{field} advertised but bound by no sender"
            );
        }
    }

    #[test]
    fn branding_is_bound_for_every_type_and_headerless_without_it() {
        // kanban t_c06a32eb. `brand_name` / `logo_url` are advertised for every type, so they must
        // be bound on every send path even for an account with no branding — otherwise an
        // admin-authored `{{brand_name}}` reaches the recipient as literal text.
        let v = bound_vars(vars(&[("name", "Ada")]));
        assert_eq!(v.get("brand_name").copied(), Some(APP_NAME));
        assert_eq!(v.get("logo_url").copied(), Some(""));
        let out = render("Hi {{name}} — from {{brand_name}} {{logo_url}}!", &v);
        assert!(unsubstituted(&out).is_empty(), "{out}");

        // A branded account: the header goes on TOP of the HTML, the text part gets the name.
        let b = crate::branding::Branding {
            brand_name: "Giraudy Capital".to_string(),
            brand_color: "#0ea5e9".to_string(),
            logo_url: "/api/v1/branding/logo/abc?v=7".to_string(),
        };
        let (text, html) = with_branding_header(
            Some(&b),
            Some("Body".to_string()),
            Some("<p>Body</p>".to_string()),
        );
        let html = html.unwrap();
        // The header OPENS the message body, but it is no longer the first byte of the string: it
        // now sits inside a real document (see `branding::place_header`, live measurement
        // 2026-11-01) instead of in front of a fragment. Assert the document and the order.
        assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
        let body_at = html.find("<body").expect("a document must carry a body");
        let head_at = html
            .find("<div style=\"text-align:center")
            .expect("the branding header must be painted");
        assert!(
            head_at > body_at,
            "the header must be inside <body>: {html}"
        );
        assert!(html.contains("<p>Body</p>"), "the original body survives");
        assert!(html.contains("https://app.funnelswift.net/api/v1/branding/logo/abc?v=7"));
        assert!(html.contains("Giraudy Capital"));
        assert_eq!(text.unwrap(), "Giraudy Capital\n\nBody");

        // ...and an account with NO branding is untouched, byte for byte.
        let (text, html) = with_branding_header(
            None,
            Some("Body".to_string()),
            Some("<p>Body</p>".to_string()),
        );
        assert_eq!(text.unwrap(), "Body");
        assert_eq!(html.unwrap(), "<p>Body</p>");
    }

    #[test]
    fn a_text_only_branded_message_gains_an_html_part_carrying_the_logo() {
        // The inline arms return `text` only. Branding that never appears is not branding, so an
        // HTML part is built — with the text ESCAPED into it, never injected as markup.
        let b = crate::branding::Branding {
            brand_name: "Acme".to_string(),
            brand_color: String::new(),
            logo_url: "/api/v1/branding/logo/abc?v=1".to_string(),
        };
        let (_, html) = with_branding_header(Some(&b), Some("<b>hi</b>".to_string()), None);
        let html = html.expect("an html part is built so the logo can render");
        assert!(html.contains("<img"));
        assert!(html.contains("&lt;b&gt;hi&lt;/b&gt;"));
        assert!(!html.contains("<b>hi</b>"));
    }

    #[test]
    fn an_unbranded_tenant_still_gets_the_apps_own_wordmark() {
        // 2026-10-24. The header block used to appear only once a tenant had uploaded a logo, so the
        // credentials mail a brand-new customer received — the first thing this product ever shows
        // them — arrived with no wordmark at all. The app's OWN identity is the fallback, and its
        // logo is a PNG on a public host (mail clients do not render SVG), with `alt` carrying the
        // name so a blocked image still says whose mail this is.
        let b = app_default_branding();
        assert_eq!(b.brand_name, APP_NAME);
        let url = b.resolve_logo_url(APP_URL).expect("the app logo resolves");
        assert_eq!(url, "https://app.funnelswift.net/logo.png");
        let header = b.header_html(Some(&url));
        assert!(header.contains("<img"), "{header}");
        assert!(header.contains(&url), "{header}");
        assert!(header.contains("alt=\"FunnelSwift\""), "{header}");
        // The colour rule still paints when the tenant chose none of their own.
        assert!(
            header.contains(crate::branding::DEFAULT_BRAND_COLOR),
            "{header}"
        );
    }

    #[test]
    fn inline_credentials_arm_matches_the_shipped_row() {
        // kanban t_36b55ed2. The authoritative copy of the new-account mail is the seeded
        // `email_templates` row; `get_inline` is its fallback. The two used to disagree on the
        // SUBJECT ("Your FunnelSwift login details" vs "Your {{app_name}} account is ready"), so the
        // same signup produced two different-looking onboarding mails depending on whether the row
        // existed. The row's subject, rendered, is the string this asserts.
        let v = bound_vars(vars(&[
            ("name", "Ada Lovelace"),
            ("email", "ada@example.com"),
            ("password", "S3cret-pw!"),
        ]));
        let (subject, body, _) = get_inline("credentials", &v);
        assert_eq!(subject, "Your FunnelSwift account is ready");
        let body = body.expect("the credentials arm is a text body");
        assert!(
            unsubstituted(&body).is_empty(),
            "placeholder left on the wire: {:?}",
            unsubstituted(&body)
        );
        // Everything the customer needs to sign in, on the wire.
        assert!(body.contains("Ada Lovelace"), "{body}");
        assert!(body.contains("ada@example.com"), "{body}");
        assert!(body.contains("S3cret-pw!"), "{body}");
        assert!(body.contains(APP_LOGIN_URL), "{body}");
    }

    #[test]
    fn shipped_welcome_row_renders_with_nothing_left_over() {
        // The live row as rewritten (kanban t_2349e6ce).
        let tpl = "Welcome to {{app_name}}, {{name}}!\n\nYour Login Credentials:\nEmail: {{email}}\n\nLogin: {{login_url}}\n\nBest,\nThe {{app_name}} Team";
        let v = bound_vars(vars(&[
            ("name", "PW Test"),
            ("email", "pwtest555@yahoo.com"),
        ]));
        let out = render(tpl, &v);
        assert!(
            unsubstituted(&out).is_empty(),
            "placeholder left on the wire: {:?}",
            unsubstituted(&out)
        );
        assert!(out.contains("PW Test"));
        assert!(out.contains("pwtest555@yahoo.com"));
        assert!(out.contains(APP_LOGIN_URL));
        assert!(!out.contains('{'));
    }
}
