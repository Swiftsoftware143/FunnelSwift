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
pub const APP_URL: &str = "https://app.funnelswift.net";

/// The private inbox's own mail client (kanban t_5592e079). Served by the `inbox.funnelswift.net`
/// vhost (`/opt/swift/nginx/sites/mail-funnelswift.conf`) from the same client every Swift app
/// serves; links in saved signatures and reminders point here.
pub const APP_URL_MAIL: &str = "https://inbox.funnelswift.net";
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

/// What the one send funnel actually did with a message.
///
/// `Ok(())` is not the same claim as "the message went out": the funnel deliberately WITHHOLDS a
/// message for a fleet-harness recipient, for an address on the workspace's do-not-mail list, and
/// (in `deliver`) for a reserved address — recording the non-send and answering `Ok` so the caller
/// does not retry. A caller that shows a human "it is on its way" therefore has to ask WHICH
/// happened, not whether the call errored. This type is that answer (2026-11-22; the same shape
/// ADASwift shipped for its Discover blast and plan notice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Handed to the provider (accepted by it, or queued by it). The only arm that may be reported
    /// to a person as "sent".
    Queued,
    /// Deliberately not attempted, with the reason and a machine-readable code.
    Withheld { code: &'static str, reason: String },
}

/// Send an already-rendered email through the tenant's DB-configured provider, reporting whether it
/// was actually handed over.
/// Graceful degradation: unconfigured logs a warning and skips (never panics, never
/// silently uses a server-wide env credential).
async fn dispatch_for_tenant_outcome(
    pool: &PgPool,
    tenant_id: Option<Uuid>,
    to: &str,
    subject: &str,
    body: Option<&str>,
    html: Option<&str>,
    reply_to: Option<&str>,
) -> Result<SendOutcome, String> {
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

    let outcome =
        crate::email_provider::deliver(&cfg, to, subject, body.unwrap_or(""), html, reply_to).await;
    // `deliver` answers Ok(()) for a reserved/harness recipient — it deliberately attempts nothing —
    // so Ok alone is not a delivery. Recording that as accepted is the misreport this line exists to
    // prevent; the withheld case is recorded as the non-send it is (2026-10-14, same class fixed on
    // ADASwift and MissedCall the same day). It is ALSO returned as `Withheld` since 2026-11-22, so
    // the caller can stop reporting a non-send as a send.
    match (&outcome, crate::security::probe_addr::harness_domain(to)) {
        (Ok(()), Some(domain)) => {
            tracing::info!(to = %to, domain = %domain, "send withheld — recording a non-send");
            // THE SUBJECT, not a generic label (2026-11-06). This arm used to record every
            // withheld send as "transactional email (withheld)", which threw away the one fact the
            // operator needs when a real mail is suppressed by mistake: WHICH mail it was. Callers
            // that suppress at a handler level already pass a meaningful kind ("signup credentials
            // email"), so the fan-out path was the odd one out. `subject` is now the kind, and the
            // history reads "<the subject> (withheld)".
            let reason = format!("'{domain}' is a fleet-harness / reserved address");
            record_withheld(pool, subject, to, &reason).await;
            Ok(SendOutcome::Withheld {
                code: "suppressed",
                reason,
            })
        }
        (Ok(()), None) => {
            record_last_send(pool, to, subject, true, "").await;
            Ok(SendOutcome::Queued)
        }
        (Err(e), _) => {
            tracing::warn!(provider = %cfg.provider, to = %to, "email send failed: {e}");
            record_last_send(pool, to, subject, false, e).await;
            Err(e.clone())
        }
    }
}

/// The old contract: "nothing went wrong". A thin wrapper over
/// [`dispatch_for_tenant_outcome`], so every existing caller is behaviourally unchanged.
async fn dispatch_for_tenant(
    pool: &PgPool,
    tenant_id: Option<Uuid>,
    to: &str,
    subject: &str,
    body: Option<&str>,
    html: Option<&str>,
) -> Result<(), String> {
    dispatch_for_tenant_outcome(pool, tenant_id, to, subject, body, html, None)
        .await
        .map(|_| ())
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
    // An account that uploaded a LOGO but never typed a brand name is the common case — the logo is
    // one upload, the name is a separate text field — and it used to render a header image with
    // `alt=""` plus a plain-text part with NO header line at all (measured live 2026-11-14 on this
    // app's own `GET /api/v1/settings/branding/preview-body` for a tenant in exactly that state:
    // `alt=""`, text starting `Hi <name>,`). Outlook blocks images by default, so that customer's
    // first mail arrived with nothing saying who sent it — while `{{brand_name}}` in the same render
    // already fell back to the app's name. Fall back ONCE, here, so the image alt, the plain-text
    // header and the merge map cannot disagree.
    let named;
    let b: &crate::branding::Branding = if b.brand_name.is_empty() {
        named = crate::branding::Branding {
            brand_name: APP_NAME.to_string(),
            ..b.clone()
        };
        &named
    } else {
        b
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
    send_template_email_for_tenant_outcome(pool, tenant_id, aid, to, template_type, vars, None)
        .await
        .map(|_| ())
}

/// [`send_template_email_for_tenant`], answering WHICH thing happened — the only form a caller may
/// use when it then tells a human "it is on its way".
pub async fn send_template_email_for_tenant_outcome(
    pool: &PgPool,
    tenant_id: Uuid,
    aid: Uuid,
    to: &str,
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
    reply_to: Option<&str>,
) -> Result<SendOutcome, String> {
    let mail_class = crate::compliance::class_of(template_type);
    let (final_subject, final_body, final_html) =
        render_template(pool, aid, template_type, vars).await;

    // The do-not-mail gate (kanban B214). AFTER the render, so the recorded non-send carries the
    // real subject; BEFORE dispatch, so no provider request is made at all. Transactional mail is
    // never gated: somebody who opted out of notices must still be able to receive a reset link.
    // A hard bounce or a spam complaint is recorded in class `all`, which stops every class — that
    // is harm control, not preference (see `blocking_classes`). Since 2026-11-22 the withheld arm
    // answers `Withheld` instead of a bare `Ok`, because a caller reporting "sent" here was
    // measured lying to the operator about a customer's sign-in help.
    if let Some(reason) = crate::compliance::gate(pool, tenant_id, to, mail_class).await {
        tracing::info!(
            to = %to,
            reason = %reason,
            "send withheld — recipient is on this workspace's do-not-mail list"
        );
        let detail = format!("recipient is on the do-not-mail list ({reason})");
        record_withheld(pool, &final_subject, to, &detail).await;
        return Ok(SendOutcome::Withheld {
            code: "do_not_mail",
            reason: detail,
        });
    }

    // Notification mail carries a working opt-out link for THIS recipient.
    let (final_body, final_html) = if mail_class == crate::compliance::NOTIFICATION {
        crate::compliance::with_unsubscribe_footer(tenant_id, to, final_body, final_html)
    } else {
        (final_body, final_html)
    };

    dispatch_for_tenant_outcome(
        pool,
        Some(tenant_id),
        to,
        &final_subject,
        final_body.as_deref(),
        final_html.as_deref(),
        reply_to,
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
        // A PERSON MOVED THE CUSTOMER ONTO A PLAN (2026-11-12). Self-serve checkout needs a payment
        // provider and no install in this fleet has one yet, so nearly every paid seat is given BY
        // HAND from the operator console. That switch used to be silent: the tier changed and the
        // customer's only clue was a screen whose allowances had moved. This arm is what the
        // operator's assign action sends once the plan has actually moved; nothing is charged, so it
        // says so, and it points at the plan screen rather than the marketing site.
        // THE CUSTOMER'S OWN RECORD OF A CLOSURE (2026-11-16). The console says "your account is
        // closed" once and then drops the session, so without this mail there is no lasting proof
        // the owner can look at later — and no way for the owner to learn that somebody else closed
        // it. It reassures the legitimate case and raises the alarm in the other one.
        "account_closed" => {
            let workspace = vars.get("workspace").copied().unwrap_or("your workspace");
            let subject = format!("Your {APP_NAME} account is closed");
            let text = format!(
                "Hi {name},\n\nThis confirms that the {APP_NAME} account for {workspace} has been closed. Every signed-in session on it has been ended and every API key on it has been revoked.\n\nNothing further will be charged, and you can sign up again at any time with this address.\n\nIf you did NOT ask for this, reply to this message straight away: closing an account needs somebody signed in to it together with its password, so this would mean another person had both.\n\n- The {APP_NAME} Team"
            );
            (subject, Some(text), None)
        }
        "plan_changed" => {
            let subject = format!("Your {APP_NAME} plan is now {plan_name}");
            let text = format!(
                "Hi {name},\n\nYour workspace has been moved onto the {plan_name} plan. Our team made this change for you, and nothing was charged to your card for it.\n\nOpen {app_url}/dashboard/ and choose Plan & Billing to see exactly what the plan includes and when it renews. If you expected a different plan, reply to this message and we will put it right.\n\n- The {APP_NAME} Team"
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
            // HOW TO PAY (2026-11-16). While no card provider is connected this request IS the
            // buying path, and until now it ended at "a person will reply" — a customer ready to
            // send money had nowhere to send it. When the operator has written his own payment
            // details, they appear here word for word; when he has not, the mail is exactly what it
            // was before, so an install that never sets them reads nothing new.
            let how = vars.get("how_to_pay").map(|s| s.trim()).unwrap_or("");
            let pay_block = if how.is_empty() {
                String::new()
            } else {
                format!("How to pay now:\n\n{how}\n\n")
            };
            let text = format!(
                "Hi {0},\n\nThanks - your request for the {1} plan is with our team. You asked for the upgrade and nothing has been charged.\n\n{2}{5}A person will reply to this address, and the answer appears on your Upgrade tab: {3}/upgrade\n\nIf you change your mind, you can withdraw the request from that same screen until we answer it.\n\n- The {4} Team",
                name, plan, note_block, app_url, APP_NAME, pay_block
            );
            (subject, Some(text), None)
        }
        // THE RECEIPT FOR AN ORDINARY HELP REQUEST (2026-11-16). The plan arm above answers a
        // customer who asked to BUY; this one answers a customer who asked for HELP - "I cannot
        // sign in", "my form stopped collecting". Until now that message wrote the row and told
        // the operator, and the person who wrote in got nothing back: no written proof their
        // message had reached anyone, so silence was indistinguishable from a lost request. It
        // promises no answer time, because no person has read it yet. `subject_line` is the
        // customer's own subject, so the mail is recognisable in an inbox list. A template row
        // named `support_request_receipt` overrides this arm.
        "support_request_receipt" => {
            let asked = vars
                .get("subject_line")
                .unwrap_or(&"your message")
                .to_string();
            let subject = format!("We have your message - {APP_NAME}");
            let text = format!(
                "Hi {0},\n\nThanks - your message reached us:\n\n    {1}\n\nA person (not a robot) reads every message and answers this address. You can follow it on the Support screen in your account: {2}/app#support\n\nIf you meant to add something, send another request from that same screen - it lands in the same queue.\n\n- The {3} Team",
                name, asked, app_url, APP_NAME
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
        // THE STORAGE RUNG REMINDER (kanban t_63c65a5c; parity with WorkflowSwift's arm, itself
        // parity with CoreSwift's B206 arm). This app carries NO `storage_warning` row in
        // `email_templates`, so without this branch the 50/75/99 mail that `evaluate_and_notify`
        // claims each month reached the customer as the generic "FunnelSwift Notification" — for
        // the one message whose entire value is its numbers. Plain sentences on purpose: the
        // subject names the real percentage because a customer reads that first, and the body
        // NEVER says mail will be deleted to make room, because keeping that promise is what the
        // whole storage rule exists for.
        "storage_warning" => {
            let used = vars.get("used").unwrap_or(&"some");
            let quota = vars.get("quota").unwrap_or(&"your plan allowance");
            let percent = vars
                .get("percent")
                .and_then(|p| p.trim().parse::<i64>().ok())
                .unwrap_or(0);
            let threshold = vars
                .get("threshold")
                .and_then(|t| t.trim().parse::<i64>().ok())
                .unwrap_or(50);
            let inbox_url = vars.get("inbox_url").unwrap_or(&APP_URL_MAIL);
            let plans_url = vars.get("plans_url").unwrap_or(app_url);
            let retention = vars.get("retention").unwrap_or(&"");
            let subject = format!("Your mailbox is {percent}% full — {used} of {quota} used");
            let action = if percent >= 100 {
                "Your mailbox is full. Mail is still arriving and nothing has been deleted, but new \
                 attachments and files are paused until you clear space or upgrade."
            } else if threshold >= 99 {
                "Your mailbox is almost full. Plain mail still arrives and nothing is deleted, but \
                 once the allowance is reached new attachments and files are paused."
            } else {
                "You have room for now. Clearing out mail you no longer need (Trash and Spam count \
                 too) keeps the meter down, and a bigger plan raises the allowance."
            };
            let text = format!(
                "Hi {name},\n\nThis is a storage update for your mailbox: you are using {used} of \
                 {quota} ({percent}%).\n\n{action}\n\nTwo things that are ALWAYS true here: mail sent \
                 to you keeps arriving, and nothing is deleted to make room for new mail. The only \
                 thing a full mailbox pauses is adding new attachments or files.\n\nOpen your mail and \
                 clear some space:\n    {inbox_url}\n\nSee the plans with more storage (Plan & Billing):\n    \
                 {plans_url}\n\n{retention}\n\nYou will get at most one of these messages per storage \
                 level per month.\n\n- The FunnelSwift Team"
            );
            // The HTML part carries the two doors as real links, with the customer's name and the
            // already-rendered sentences escaped (the retention sentence is account data).
            let esc = |s: &str| {
                s.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            };
            let safe_name = esc(name);
            let safe_action = esc(action);
            let safe_retention = esc(retention);
            let html = format!(
                "<html><body style=\"font-family:Arial,sans-serif;line-height:1.6;color:#333\">\
                 <p>Hi {safe_name},</p>\
                 <p>This is a storage update for your mailbox: you are using <strong>{used} of {quota} ({percent}%)</strong>.</p>\
                 <p>{safe_action}</p>\
                 <p>Two things that are <strong>always</strong> true here: mail sent to you keeps arriving, and <strong>nothing is deleted</strong> to make room for new mail. The only thing a full mailbox pauses is adding new attachments or files.</p>\
                 <p><a href=\"{inbox_url}\" style=\"color:#4f46e5\">Open your mail and clear some space</a></p>\
                 <p><a href=\"{plans_url}\" style=\"color:#4f46e5\">See the plans with more storage (Plan &amp; Billing)</a></p>\
                 <p style=\"font-size:13px;color:#9ca3af\">{safe_retention}<br>You will get at most one of these messages per storage level per month.<br>- The FunnelSwift Team</p></body></html>"
            );
            (subject, Some(text), Some(html))
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

/// The welcome mail the portfolio-sync door sends a brand-new workspace (fleet portfolio
/// provisioning standard, 2026-11-15): the login (email + the DERIVED password), the app name, and
/// the workspace's API key PREFIX only — never the full key. The full key is viewable and copyable
/// in-app in one click from a signed-in console (`POST /api/v1/api-keys/:id/reveal`), exactly as a
/// manual signup works.
///
/// Composed inline rather than through the `credentials` template so the sync door can carry the
/// one extra fact a manual signup's mail has no need for (the key prefix) without changing what a
/// real signup customer receives. Sent through the tenant's own provider (falling back to system
/// mail) exactly like every other transactional send; a reserved/harness recipient is withheld by
/// `dispatch_for_tenant` → `deliver`.
pub async fn send_portfolio_provision_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    name: &str,
    password: &str,
    key_prefix: &str,
) -> Result<(), String> {
    let subject = format!("Your {APP_NAME} account is ready");
    let text = format!(
        "Hi {name},\n\nYour {APP_NAME} workspace is ready to use.\n\nSign in at {APP_LOGIN_URL} with:\nEmail: {to}\nPassword: {password}\n\nThe password above is temporary. After you sign in, go to your profile settings and replace it with one of your own.\n\nYour workspace API key starts with: {key_prefix}\nThe full key lives in your account under Settings > API keys, where one click shows it or copies it.\n\n- The {APP_NAME} Team"
    );
    let html = format!(
        "<p>Hi {name},</p><p>Your {APP_NAME} workspace is ready to use.</p><p>Sign in at <a href=\"{APP_LOGIN_URL}\">{APP_LOGIN_URL}</a> with:</p><p>Email: {to}<br>Password: {password}</p><p>The password above is temporary. After you sign in, go to your profile settings and replace it with one of your own.</p><p>Your workspace API key starts with: <strong>{key_prefix}</strong><br>The full key lives in your account under Settings &gt; API keys, where one click shows it or copies it.</p><p>- The {APP_NAME} Team</p>"
    );
    dispatch_for_tenant(
        pool,
        Some(tenant_id),
        to,
        &subject,
        Some(&text),
        Some(&html),
    )
    .await
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
    send_reset_email_outcome(pool, tenant_id, to, token, name)
        .await
        .map(|_| ())
}

/// [`send_reset_email`], answering whether the message was actually handed to the provider.
///
/// The operator's "send sign-in help" route shows a person "it is on its way" on this mail, so it
/// must not read a withheld non-send as a send (2026-11-22).
pub async fn send_reset_email_outcome(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    token: &str,
    name: &str,
) -> Result<SendOutcome, String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("token", token);
    send_template_email_for_tenant_outcome(
        pool,
        tenant_id,
        tenant_id,
        to,
        "password_reset",
        &bound_vars(vars),
        None,
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
    request_id: Uuid,
    to: &str,
    name: &str,
    plan_name: &str,
    note: &str,
    how_to_pay: &str,
) -> Result<bool, String> {
    // ANSWERS WHETHER THE RECEIPT REALLY WENT OUT (2026-11-20). It used to answer `Ok(())` for every
    // suppressed leg too, so `acknowledged` was TRUE for a mail that never left: measured live on this
    // app, a request filed by a reserved address answered `acknowledged: true` and the Upgrade screen
    // rendered "We have emailed you a copy of this request." while `email_send_log` carried the row as
    // `withheld`. The screen promises what the sender knows — so the sender now says it.
    //
    // THE OPERATOR'S OWN REQUESTS MUST NOT MAIL THE OPERATOR (2026-11-18). A platform admin
    // (David) filing from his own workspace would otherwise receive this receipt about a request he
    // filed himself. The row still lands in the queue and the request still works; only the receipt
    // mail to the requester is withheld.
    if requester_is_platform_admin(pool, to).await {
        tracing::info!(
            requester = %to,
            "operator self-request: receipt suppressed"
        );
        return Ok(false);
    }
    // A PROBE-MARKED WORKSPACE NEVER MAILS A CUSTOMER (kanban t_a14ad547). The incident this card
    // is about mailed a real customer a receipt the operator never saw; that tenant was a customer's,
    // so no gate fired. This is the other half: when the WORKSPACE is harness-minted
    // (`tenants.probe_harness`), the receipt is machinery talking to itself — withheld, never sent,
    // and never stamped as acknowledged. A database error withholds (fail closed).
    match crate::probe_harness::tenant_is_probe(pool, tenant_id).await {
        Ok(true) => {
            tracing::info!(
                tenant = %tenant_id,
                to = %to,
                "plan-request receipt WITHHELD — the workspace is harness-minted"
            );
            return Ok(false);
        }
        Ok(false) => {}
        Err(e) => {
            return Err(format!(
                "plan request {request_id}: could not tell whether the workspace is a probe \
                 workspace, so the receipt was not sent: {e}"
            ))
        }
    }
    // A RESERVED / FLEET-HARNESS RECIPIENT IS A NON-SEND, not a receipt. `dispatch_for_tenant`
    // records it as `withheld` and attempts no provider request at all, so reporting it as sent is
    // the same lie one layer down. The verdict below reads the SAME predicate the dispatcher used, so
    // the two cannot disagree — and the dispatcher still writes its own `withheld` row with the real
    // subject, which is the history the operator needs.
    //
    // AN ACKNOWLEDGEMENT IS BOUND TO A COMMITTED ROW BEFORE THE MAIL LEAVES (2026-11-26).
    // Measured: a live probe filed request 6c51df70… for a real customer, both the receipt and the
    // operator alert went out, and the probe deleted the row a minute later. The customer held a
    // written "we have your request" while no row existed anywhere. Ordering alone cannot fix that:
    // the row was committed before the mail. The durable fix is `plan_requests.acknowledged_at`,
    // stamped HERE and BEFORE the send, plus a database trigger that refuses to delete a stamped
    // row (migration 148). A failed stamp therefore refuses the send — never the other way round.
    let will_send = crate::security::probe_addr::harness_domain(to).is_none();
    if will_send {
        let marked = sqlx::query(
            "UPDATE plan_requests SET acknowledged_at = COALESCE(acknowledged_at, now()) \
              WHERE id = $1 AND tenant_id = $2",
        )
        .bind(request_id)
        .bind(tenant_id)
        .execute(pool)
        .await
        .map_err(|e| {
            format!("plan request {request_id}: could not stamp the acknowledgement: {e}")
        })?;
        if marked.rows_affected() == 0 {
            return Err(format!(
                "plan request {request_id} is not on record for this workspace; the receipt was not sent"
            ));
        }
    }
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("plan_name", plan_name);
    vars.insert("note", note);
    // The operator's own payment details, when he has written any (2026-11-16). Always bound, so a
    // template that references `{{how_to_pay}}` cannot send the placeholder literally.
    vars.insert("how_to_pay", how_to_pay);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "plan_request_received",
        &bound_vars(vars),
    )
    .await?;
    Ok(will_send)
}

/// Acknowledge an ORDINARY support request the moment the customer files it (2026-11-16).
///
/// The plan-request receipt above answers "I want to buy"; this one answers "help me" — "I cannot
/// sign in", "my form stopped collecting". Until this existed the request wrote its row and mailed
/// the operator, and the person who wrote in got nothing at all, so they could not tell a request
/// that reached the team from one that never left the browser. The same single send funnel as every
/// other mail here: the tenant's own template row if it has one, per-account branding, and the
/// reserved/harness-address gate — a probe address is recorded as a `withheld` non-send and no
/// message is attempted.
///
/// Best effort by design: the row is the record, so a mail failure must never lose a filed request.
/// The caller reports the outcome as `acknowledged`, so the console never promises a receipt that
/// did not happen.
pub(crate) async fn send_support_request_receipt_email(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    name: &str,
    subject_line: &str,
) -> Result<bool, String> {
    // Same rule as the plan-request receipt above (2026-11-20): the caller's `acknowledged` is only
    // TRUE when something was really handed to the provider, never for a suppressed or withheld leg.
    //
    // THE OPERATOR'S OWN REQUESTS MUST NOT MAIL THE OPERATOR (2026-11-18). A platform admin
    // (David) filing from his own workspace would otherwise receive this receipt about a request he
    // filed himself. The row still lands in the queue and the request still works; only the receipt
    // mail to the requester is withheld.
    if requester_is_platform_admin(pool, to).await {
        tracing::info!(
            requester = %to,
            "operator self-request: receipt suppressed"
        );
        return Ok(false);
    }
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("subject_line", subject_line);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "support_request_receipt",
        &bound_vars(vars),
    )
    .await?;
    Ok(crate::security::probe_addr::harness_domain(to).is_none())
}

/// Confirm to the CUSTOMER that their workspace is closed (2026-11-16).
///
/// WHY THIS EXISTS: `POST /api/v1/auth/close-account` answered 200, ended every session and revoked
/// every API key — and told nobody. The owner who pressed the button got no record that the account
/// was closed, and an owner whose account was closed by somebody else (a stolen session, a shared
/// laptop) got no signal at all: the next thing they saw was "password incorrect". A closure is the
/// most consequential thing that happens to an account short of deletion, so it is confirmed by mail
/// to the very address the closure keeps refusing.
///
/// The same single send funnel as every other mail here: the workspace's own stored template row if
/// it has one, per-account branding, and the reserved/harness-address gate — a probe address is
/// recorded as a `withheld` non-send and no message is attempted.
///
/// Best effort by design: the closure is already committed, so a mail failure is logged by the caller
/// and never turned into an error.
pub(crate) async fn send_account_closed_confirmation(
    pool: &PgPool,
    tenant_id: Uuid,
    to: &str,
    name: &str,
    workspace: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("workspace", workspace);
    send_template_email_for_tenant(
        pool,
        tenant_id,
        tenant_id,
        to,
        "account_closed",
        &bound_vars(vars),
    )
    .await
}

/// Tell the OPERATOR that a customer closed their own account (2026-11-16).
///
/// WHY THIS EXISTS: the operator is mailed for a new workspace, a plan request, a support request, a
/// customer whose first password never arrived and a customer who is locked out — but a customer
/// LEAVING, the one event a subscription business cannot afford to hear about late, was silent. The
/// workspace simply stopped appearing, and nothing distinguished "cancelled" from "a quiet week".
///
/// Recipient = the operator's own `operator_alert_email` (Admin -> System Settings). An unset address
/// sends nothing; this code never invents a mailbox. Sent through the SYSTEM mail configuration
/// (`None`), because this is platform mail and must arrive even when the leaving workspace's own mail
/// path is the thing that is broken.
///
/// Never sent for a probe/harness identity: the gate is on the REQUESTER (the owner identity and the
/// workspace name), never on the route, because the same door serves real customers. A harness that
/// closes its own probe account must not tell the operator that a customer cancelled.
pub async fn send_account_closed_operator_alert(
    pool: &PgPool,
    owner_name: &str,
    owner_email: &str,
    workspace: &str,
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
    let synthetic = crate::security::probe_addr::synthetic_requester(owner_email, owner_name)
        .or_else(|| crate::security::probe_addr::synthetic_requester("", workspace));
    if let Some(reason) = synthetic {
        tracing::info!(
            reason = %reason,
            "account-closed operator alert WITHHELD — the closed account is a synthetic identity"
        );
        return Ok(());
    }
    let who = if owner_name.trim().is_empty() {
        owner_email.to_string()
    } else {
        format!("{} ({})", owner_name.trim(), owner_email)
    };
    let workspace = if workspace.trim().is_empty() {
        owner_email.to_string()
    } else {
        workspace.trim().to_string()
    };
    let cancelled_on = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let subject = format!("Customer cancelled: {workspace}");
    let text = format!(
        "A customer closed their own account.\n\n\
         Workspace: {workspace}\n\
         Owner: {who}\n\
         Closed on: {cancelled_on}\n\n\
         Nothing was charged for the closure. The account can no longer sign in, every session it \
         had is dead and every API key on it has been revoked. Its data is kept, so the workspace, \
         its leads and its figures are still in the admin console if they come back or ask about it."
    );
    let html = format!(
        "<p>A customer closed their own account.</p>\
         <p><strong>{}</strong><br>\
         Owner: {}<br>\
         Closed on: {}</p>\
         <p>Nothing was charged for the closure. The account can no longer sign in, every session it \
         had is dead and every API key on it has been revoked. Its data is kept, so the workspace, \
         its leads and its figures are still in the admin console if they come back or ask about it.</p>",
        crate::templates::html_escape(&workspace),
        crate::templates::html_escape(&who),
        crate::templates::html_escape(&cancelled_on),
    );
    dispatch_for_tenant(pool, None, to, &subject, Some(&text), Some(&html)).await
}

/// Is the requester a platform admin? (2026-11-18)
///
/// The operator's OWN account is a workspace in its own product. When David — or any platform
/// admin — files a plan or support request from his own workspace, the request door mails
/// `operator_alert_email`, which is his own address, so he receives "A customer asked to buy X"
/// about a request he filed himself. The row still lands in the queue and the request still works;
/// only the ALERT to a human is withheld. A lookup failure answers `false` (send the alert): a real
/// customer's alert must never be lost because this probe tripped.
async fn requester_is_platform_admin(pool: &PgPool, email: &str) -> bool {
    let Ok(roles) = sqlx::query_scalar::<_, String>(
        "SELECT role FROM users WHERE lower(email) = lower($1) AND is_active = true",
    )
    .bind(email.trim())
    .fetch_all(pool)
    .await
    else {
        return false;
    };
    roles
        .iter()
        .any(|role| crate::auth::global_auth::is_platform_admin(role))
}

/// The operator alert's words, in one place so a test can read them without a database.
///
/// THE REQUEST ID IS PART OF THE ALERT (2026-11-26). The alert used to say "waiting in the admin
/// console" and nothing else. Measured the next morning: a probe had deleted the row a minute after
/// mailing, so the operator was pointed at a queue entry that no longer existed and had no way to
/// tell. The id makes the promise checkable — the console prints it on the row, so "not in the queue"
/// is visible instead of silent.
fn plan_request_operator_alert_copy(
    workspace: &str,
    plan_name: &str,
    requester_email: &str,
    note: &str,
    request_id: Uuid,
) -> (String, String, String) {
    let subject = format!("A customer asked to buy {plan_name}");
    let text = format!(
        "{workspace} asked to be moved onto {plan_name}.\n\n\
         Request ID: {request_id}\n\
         Requested by: {requester_email}\n\
         Plan: {plan_name}\n\
         Note from the customer: {note}\n\n\
         Find this request by its ID in the admin console (Plans -> Plan requests); the queue \
         prints the request ID on the row. Moving it to done puts that workspace on the plan."
    );
    let html = format!(
        "<p><strong>{}</strong> asked to be moved onto <strong>{}</strong>.</p>\
         <p>Request ID: <code>{}</code></p>\
         <p>Requested by: {}</p>\
         <p>Note from the customer: {}</p>\
         <p>Find this request by its ID in the admin console (Plans &rarr; Plan requests); the \
         queue prints the request ID on the row. Moving it to <em>done</em> puts that workspace on \
         the plan.</p>",
        crate::templates::html_escape(workspace),
        crate::templates::html_escape(plan_name),
        request_id,
        crate::templates::html_escape(requester_email),
        crate::templates::html_escape(note),
    );
    (subject, text, html)
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
    request_id: Uuid,
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
    // A PROBE-MARKED WORKSPACE NEVER RINGS THE OPERATOR (kanban t_a14ad547). The requester predicate
    // above reads the address and the workspace NAME, so machinery with a customer-shaped name got
    // through: measured 2026-11-26, a probe filed a request for the real EMI Mold Removal workspace
    // and the operator was mailed "a customer asked to buy". The workspace's own stamp is data, not
    // a guess — when it says harness, the human is not paged.
    match crate::probe_harness::tenant_is_probe(pool, tenant_id).await {
        Ok(true) => {
            tracing::info!(
                tenant = %tenant_id,
                requester = %requester_email,
                "plan-request operator alert WITHHELD — the workspace is harness-minted"
            );
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => {
            return Err(format!(
                "plan_request {request_id}: probe verdict failed: {e}"
            ))
        }
    }
    // THE OPERATOR'S OWN REQUESTS MUST NOT MAIL THE OPERATOR (2026-11-18). A platform admin
    // (David) filing from his own workspace would otherwise mail `operator_alert_email` — his own
    // address — "A customer asked to buy X" about a request he filed himself. The row still lands
    // in the queue; only the alert to a human is withheld.
    if requester_is_platform_admin(pool, requester_email).await {
        tracing::info!(
            requester = %requester_email,
            "operator self-request: alert suppressed"
        );
        return Ok(());
    }
    let note = if note.trim().is_empty() {
        "(none)"
    } else {
        note.trim()
    };
    let (subject, text, html) =
        plan_request_operator_alert_copy(&workspace, plan_name, requester_email, note, request_id);
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
    // A FLEET PROBE MUST NEVER RING THE OPERATOR (kanban t_f17b5802, extended 2026-11-13).
    // The plan-request and support-request arms gate on `synthetic_requester`; this arm gated only
    // on the MAIL-suppression class, and `maxxspace.com` is deliberately NOT in that class (the
    // credential-delivery proof reads its password back out of that inbox, so suppressing mail
    // there would delete the only end-to-end proof that a customer's password arrives). Measured
    // live 2026-11-13 19:56Z: an audit signup into the disposable inbox still delivered
    // "New workspace: Albert Tire fspre1605's Workspace" to the operator's real mailbox.
    // Requester, not route: the same door serves real customers. BOTH the owner identity and the
    // workspace name are judged, because a harness workspace can carry a clean address
    // ("Probe Co 1794597085") and the name is the only tell.
    let synthetic = crate::security::probe_addr::synthetic_requester(owner_email, owner_name)
        .or_else(|| crate::security::probe_addr::synthetic_requester("", &workspace));
    if let Some(reason) = synthetic {
        tracing::info!(
            reason = %reason,
            "signup operator alert WITHHELD — the new workspace is a synthetic identity"
        );
        return Ok(());
    }
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

/// Tell the OPERATOR that a customer's first password never reached them (2026-11-15).
///
/// WHY THIS EXISTS (measured live 2026-11-15 on the sibling app ADASwift): a real customer signed
/// up, the mail carrying their first password never left the box, and the account sat unusable for
/// two days. The `password_undelivered` latch and the console badge both existed — but a badge is
/// only read by somebody who happens to open that screen, and nobody was ever told. The operator
/// already gets a mail for a new workspace, a plan request and a support request; a customer who
/// cannot sign in is the most urgent of those four, and it was the only one with no alert.
///
/// Recipient = the operator's own `operator_alert_email` (Admin -> System Settings), passed in by
/// the caller, which resolves it once for the whole pass. Sent through the SYSTEM mail configuration
/// (`None`), so it still arrives when the customer's own mail path is the broken thing.
///
/// The SUBJECT carries the user id: that is the watch's "already reported" key, so a later pass does
/// not page the operator twice for one account. It returns the status of the row this send wrote —
/// `deliver` answers Ok for a recipient the transport WITHHOLDS, and Ok is not "the operator saw it".
pub async fn send_undelivered_credential_operator_alert(
    pool: &PgPool,
    user_id: Uuid,
    customer_email: &str,
    workspace: &str,
    owner_name: &str,
    signed_up: &str,
    to: &str,
) -> Result<String, String> {
    let who = if owner_name.trim().is_empty() {
        customer_email.to_string()
    } else {
        format!("{} ({})", owner_name.trim(), customer_email)
    };
    let workspace = if workspace.trim().is_empty() {
        customer_email
    } else {
        workspace.trim()
    };
    let signed_up = if signed_up.trim().is_empty() {
        "unknown"
    } else {
        signed_up.trim()
    };
    // THE SUBJECT MUST NAME THE CUSTOMER IN PLAIN ENGLISH (2026-11-16). It used to carry only the
    // account's uuid — "A customer cannot sign in — no password was delivered (437870e2-…)" — so the
    // operator could not tell from the notification list WHO was locked out; he had to open every one
    // to find out. `who` is the owner's name with their address (or the address alone when no name was
    // given), which is what a person recognises.
    //
    // The uuid STAYS in the subject on purpose: `credential_watch`'s "already reported" test is
    // `email_send_log.subject LIKE '%' || users.id || '%' AND to_email = operator AND status='sent'`,
    // and this function's own read-back matches the same way. Dropping the id would turn every pass
    // into a fresh report of every account — the repeated-operator-mail failure mode this fleet has
    // hit before. Keep both: the name to read, the id to de-duplicate on.
    let subject = format!("A customer cannot sign in — {who} has no password ({user_id})");
    let text = format!(
        "{workspace} signed up, but the email carrying their first password never reached them, so \
         they cannot sign in.\n\n\
         Workspace: {workspace}\n\
         Customer: {who}\n\
         Signed up: {signed_up}\n\n\
         A reset code travels the same mail path that just failed, so it may not reach them either. \
         The admin console (Tenants) shows a badge on that row and has the door that sets a first \
         password for them, which is the way back in."
    );
    let html = format!(
        "<p><strong>{}</strong> signed up, but the email carrying their first password never reached \
         them, so they cannot sign in.</p>\
         <p>Customer: {}<br>Signed up: {}</p>\
         <p>A reset code travels the same mail path that just failed, so it may not reach them \
         either. The admin console (Tenants) shows a badge on that row and has the door that sets a \
         first password for them, which is the way back in.</p>",
        crate::templates::html_escape(workspace),
        crate::templates::html_escape(&who),
        crate::templates::html_escape(signed_up),
    );
    dispatch_for_tenant(pool, None, to, &subject, Some(&text), Some(&html)).await?;
    // What the send ACTUALLY did. The withheld arm records its row with a " (withheld)" suffix, so
    // the read-back matches on the user id rather than the exact subject.
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM email_send_log WHERE to_email = $1 AND subject LIKE '%' || $2 || '%' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(to)
    .bind(user_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("read back the alert row: {e}"))?;
    Ok(status.unwrap_or_else(|| "unknown".to_string()))
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
    // THE OPERATOR'S OWN REQUESTS MUST NOT MAIL THE OPERATOR (2026-11-18). A platform admin filing
    // from his own workspace would otherwise mail `operator_alert_email` — his own address — about
    // a ticket he filed himself. The row still lands in the queue; only the alert is withheld.
    if requester_is_platform_admin(pool, from_email).await {
        tracing::info!(
            requester = %from_email,
            "operator self-request: alert suppressed"
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
    // A PROBE VISITOR MUST NEVER RING THE OWNER (2026-11-21, test-mail-hygiene 2c). This arm was
    // the last mail path in the app with no requester gate: the RECIPIENT is honoured correctly
    // (a reserved owner address is withheld by `dispatch_for_tenant`), but the TRIGGER is the
    // visitor who submitted the form, and a fleet probe submits one per walk. Measured live:
    // "New lead on ZZ Probe Funnel Capture" was delivered to the operator's own mailbox because a
    // probe capture ran against the operator's own workspace — where the owner address is real and
    // nothing withheld it. Gate on the REQUESTER (the visitor), never on the route: the same public
    // form serves real customers, so the predicate is the only thing that can tell them apart.
    // Returns BEFORE any dispatch, so no `email_send_log` row is written for a probe capture at all.
    if let Some(reason) = crate::security::probe_addr::synthetic_requester(lead_email, lead_name) {
        tracing::info!(
            reason = %reason,
            funnel = %funnel_name,
            "lead notification WITHHELD — the visitor is a synthetic identity"
        );
        return Ok(());
    }
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", owner_name);
    vars.insert("lead_name", lead_name);
    vars.insert("lead_email", lead_email);
    vars.insert("lead_phone", lead_phone.unwrap_or(""));
    vars.insert("funnel_name", funnel_name);
    // THE PROMISE THIS MAKES IS THE REASON FOR THE HEADER (2026-11-27): the body tells the owner to
    // "reply to this message to answer them directly". Until this arm passed a Reply-To the reply
    // went to the sending address (no-reply@), so the sentence described something the product did
    // not do. A lead with no address keeps the previous behaviour (no header).
    send_template_email_for_tenant_outcome(
        pool,
        tenant_id,
        tenant_id,
        to,
        "new_lead",
        &bound_vars(vars),
        Some(lead_email).filter(|e| !e.trim().is_empty()),
    )
    .await
    .map(|_| ())
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
    fn plan_request_operator_alert_carries_the_request_id_it_promises() {
        // Measured 2026-11-26: the alert said "waiting in the admin console" while the row it
        // promised was deleted a minute later, so the operator was pointed at nothing. The id in
        // the mail is what makes the console entry checkable — and the console prints it on the row.
        let id = Uuid::from_u128(0x6c51df70_4135_4018_a42a_02dcb3920727);
        let (subject, text, html) = plan_request_operator_alert_copy(
            "EMI Mold Removal Tulsa OK",
            "Agency / Scale",
            "contact@example.com",
            "please call",
            id,
        );
        assert_eq!(subject, "A customer asked to buy Agency / Scale");
        assert!(text.contains(&id.to_string()), "text was {text}");
        assert!(html.contains(&id.to_string()), "html was {html}");
        assert!(
            text.contains("prints the request ID on the row"),
            "text was {text}"
        );
        assert!(
            !text.contains("The request is waiting in the admin console"),
            "the old unnamed-queue sentence is back: {text}"
        );
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

    // HOW TO PAY (2026-11-16): with no card provider the plan request is the buying path, so the
    // receipt must carry the operator's own payment details when he has written any — and must be
    // byte-identical to before when he has not.
    #[test]
    fn plan_request_receipt_carries_the_operators_payment_instructions_when_set() {
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("plan_name", "Kinetic Pro"),
            ("note", ""),
            ("how_to_pay", "Bank transfer to Swift Software LLC, account 000111222. Quote your workspace name."),
        ]));
        let (_subject, text, _html) = get_inline("plan_request_received", &v);
        let text = text.unwrap_or_default();
        assert!(text.contains("How to pay now:"), "no pay heading: {text}");
        assert!(
            text.contains("account 000111222"),
            "the operator's own words are missing: {text}"
        );
        assert!(
            text.contains("/upgrade"),
            "where to look is missing: {text}"
        );
    }

    #[test]
    fn plan_request_receipt_shows_no_pay_block_when_none_is_configured() {
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("plan_name", "Kinetic Pro"),
            ("note", ""),
            ("how_to_pay", ""),
        ]));
        let (_subject, text, _html) = get_inline("plan_request_received", &v);
        let text = text.unwrap_or_default();
        assert!(
            !text.contains("How to pay now:"),
            "an empty setting painted a block: {text}"
        );
        assert!(text.contains("nothing has been charged"));
    }

    // The receipt an ORDINARY help request now earns (2026-11-16). Before it, only a plan request
    // was acknowledged by mail; "I cannot sign in" got a sentence on a screen and an empty inbox.
    #[test]
    fn support_request_receipt_quotes_the_request_and_names_where_it_lives() {
        let v = bound_vars(vars(&[
            ("name", "Probe"),
            ("subject_line", "I cannot sign in"),
        ]));
        let (subject, text, _html) = get_inline("support_request_receipt", &v);
        let text = text.unwrap_or_default();
        assert!(
            text.contains("I cannot sign in"),
            "the ask is not quoted: {text}"
        );
        assert!(
            text.contains("/app#support"),
            "where to look is missing: {text}"
        );
        assert!(
            subject.contains("We have your message"),
            "subject was {subject}"
        );
        // It is a receipt, not a promise: no person has read the request yet, so it must not
        // name an answer time.
        assert!(
            !text.contains("within"),
            "an answer time was promised: {text}"
        );
        assert!(!unsubstituted(&text).contains(&"name"));
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
    fn a_logo_with_no_brand_name_still_says_whose_mail_it_is() {
        // Measured live 2026-11-14 (`GET /api/v1/settings/branding/preview-body` on a tenant that
        // had uploaded a logo and typed no name): the header image carried `alt=""` and the
        // plain-text part had NO header line, so a client that blocks images — Outlook's default —
        // showed nothing identifying the sender, while `{{brand_name}}` in the same render already
        // said FunnelSwift. The three must agree.
        let b = crate::branding::Branding {
            brand_name: String::new(),
            brand_color: String::new(),
            logo_url: "/api/v1/branding/logo/abc?v=1".to_string(),
        };
        let (text, html) = with_branding_header(Some(&b), Some("Body".to_string()), None);
        assert_eq!(text.unwrap(), format!("{APP_NAME}\n\nBody"));
        let html = html.expect("an html part is built so the logo can render");
        assert!(
            html.contains(&format!("alt=\"{APP_NAME}\"")),
            "the blocked-image fallback must name the app: {html}"
        );
        assert!(!html.contains("alt=\"\""), "{html}");
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
