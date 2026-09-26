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

/// Look up + render a template (DB-backed, falling back to inline hardcoded content).
async fn render_template(
    pool: &PgPool,
    aid: Uuid,
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
) -> (String, Option<String>, Option<String>) {
    match get_db_template(pool, aid, template_type).await {
        Ok(Some((subject, body, html_body))) => {
            let subject = render(&subject, vars);
            let body = body.map(|b| render(&b, vars));
            let html = html_body.map(|h| render(&h, vars));
            (subject, body, html)
        }
        _ => get_inline(template_type, vars),
    }
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
        let advertised: [&[&str]; 3] = [
            &["name", "email", "login_url", "app_name"],
            &["name", "token", "app_name"],
            &["name", "plan_name", "login_url", "app_name"],
        ];
        let bound = bound_vars(vars(&[
            ("name", "n"),
            ("email", "e"),
            ("token", "t"),
            ("plan_name", "p"),
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
