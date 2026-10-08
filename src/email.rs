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
        return Err(
            "Email provider not configured. Set it in Admin > Settings > Email Provider."
                .to_string(),
        );
    };

    crate::email_provider::deliver(&cfg, to, subject, body.unwrap_or(""), html)
        .await
        .map_err(|e| {
            tracing::warn!(provider = %cfg.provider, to = %to, "email send failed: {e}");
            e
        })
}

/// The app's OWN support address. David (2026-10-08): the reply-to/support address in
/// a transactional mail is ALWAYS `support@` the app's own main domain — never the
/// platform's or a sibling company's. FunnelSwift mail carried none at all (no address
/// in any body), so [`with_support_footer`] adds this to every rendered message.
pub const SUPPORT_EMAIL: &str = "support@funnelswift.net";

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
        if t.contains(SUPPORT_EMAIL) {
            t
        } else {
            format!("{}\n\nNeed help? Contact {}\n", t, SUPPORT_EMAIL)
        }
    });
    let html = html.map(|h| {
        if h.contains(SUPPORT_EMAIL) {
            h
        } else {
            format!(
                "{}\n<p style=\"font-size:13px;color:#6b7280;text-align:center;\">Need help? Contact <a href=\"mailto:{}\">{}</a></p>",
                h, SUPPORT_EMAIL, SUPPORT_EMAIL
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
        Some(h) => Some(format!("{header}{h}")),
        None => text.as_deref().map(|t| {
            format!(
                "{header}<div style=\"font:14px/1.6 -apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#111827;white-space:pre-wrap\">{}</div>",
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
    let branding = crate::branding::load(pool, aid).await;
    let brand_name = branding
        .as_ref()
        .map(|b| b.brand_name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| APP_NAME.to_string());
    let logo_url = branding
        .as_ref()
        .and_then(|b| b.resolve_logo_url(APP_URL))
        .unwrap_or_default();

    let mut all: std::collections::HashMap<&str, &str> =
        std::collections::HashMap::with_capacity(vars.len() + 2);
    for (k, v) in vars {
        all.insert(k, v);
    }
    all.insert("brand_name", &brand_name);
    all.insert("logo_url", &logo_url);

    let (subject, body, html) = match get_db_template(pool, aid, template_type).await {
        Ok(Some((subject, body, html_body))) => (
            render(&subject, &all),
            body.map(|b| render(&b, &all)),
            html_body.map(|h| render(&h, &all)),
        ),
        _ => get_inline(template_type, &all),
    };

    let (body, html) = with_branding_header(branding.as_ref(), body, html);
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
            let text = format!(
                "Hi {0},\n\nWe received a request to reset your password for FunnelSwift.\n\nYour reset code is: {1}\n\nThis code expires in 1 hour.\n\nIf you did not request this, please ignore this email.\n\n- FunnelSwift Team",
                name, token
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

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &[(&'a str, &'a str)]) -> std::collections::HashMap<&'a str, &'a str> {
        pairs.iter().copied().collect()
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
        let advertised: [&[&str]; 4] = [
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
        ];
        let bound = bound_vars(vars(&[
            ("name", "n"),
            ("email", "e"),
            ("password", "pw"),
            ("token", "t"),
            ("plan_name", "pl"),
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
        assert!(html.starts_with("<div style=\"text-align:center"));
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
