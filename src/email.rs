//! Email module — sends transactional emails using database-stored templates.
//!
//! Templates are stored in `email_templates` with html_body (HTML) and body (plain text).
//! Falls back to hardcoded inline templates when DB template not found.
//!
//! Provider resolution (per-tenant → env fallback):
//!   - `send_template_email_for_tenant` resolves the tenant's Mailgun/SMTP/email config from
//!     the Integration Center (`target_software`), reading `webhook_url` as the Mailgun API
//!     base and `api_key` as the key. Falls back to EMAIL_API_URL / EMAIL_API_KEY / EMAIL_FROM
//!     env vars when the tenant has no active email-provider row.
//!   - `send_template_email` is the env-only path for system emails with no tenant context.
//!
//! Direct API send (no queue).

use serde_json::json;
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

/// Resolved email provider config (API base + key + from address).
struct ProviderConfig {
    api_url: String,
    api_key: String,
    from: String,
}

/// Render a template string by replacing {{key}} placeholders
fn render(template: &str, vars: &std::collections::HashMap<&str, &str>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{}}}}}", key), value);
    }
    result
}

/// Resolve the tenant's Mailgun/SMTP/email provider config from the Integration Center
/// (`target_software`). Returns `None` when the tenant has no active email-provider row,
/// in which case the caller falls back to the env-var provider.
async fn resolve_tenant_provider(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Option<ProviderConfig>, String> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        r#"SELECT webhook_url, api_key
           FROM target_software
           WHERE tenant_id = $1
             AND (LOWER(name) LIKE '%mailgun%' OR LOWER(name) LIKE '%smtp%' OR LOWER(name) LIKE '%email%')
             AND is_active = true
           ORDER BY created_at ASC
           LIMIT 1"#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to resolve email provider: {e}"))?;

    let Some((api_url, api_key)) = row else {
        return Ok(None);
    };

    if api_url.trim().is_empty() {
        return Ok(None);
    }

    let from = env::var("EMAIL_FROM").unwrap_or_else(|_| "swiftsoftware143@yahoo.com".to_string());

    Ok(Some(ProviderConfig {
        api_url,
        api_key: api_key.unwrap_or_default(),
        from,
    }))
}

/// System-wide provider config from env vars (no tenant context).
fn env_provider() -> Result<ProviderConfig, String> {
    let api_url = env::var("EMAIL_API_URL").map_err(|_| "EMAIL_API_URL not set".to_string())?;
    let api_key = env::var("EMAIL_API_KEY").map_err(|_| "EMAIL_API_KEY not set".to_string())?;
    let from = env::var("EMAIL_FROM").unwrap_or_else(|_| "swiftsoftware143@yahoo.com".to_string());
    Ok(ProviderConfig {
        api_url,
        api_key,
        from,
    })
}

/// Send an already-rendered email through the given provider.
async fn dispatch(
    config: &ProviderConfig,
    to: &str,
    subject: &str,
    body: Option<&str>,
    html: Option<&str>,
) -> Result<(), String> {
    let mut payload = json!({
        "from": config.from,
        "to": to,
        "subject": subject,
    });

    if let Some(obj) = payload.as_object_mut() {
        if let Some(html) = html {
            obj.insert("html".into(), json!(html));
        }
        if let Some(text) = body {
            obj.insert("text".into(), json!(text));
        }
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(&config.api_url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send email: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Email API returned {}: {}", status, text));
    }

    Ok(())
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
/// Integration Center (`target_software`) first, falling back to env vars when absent.
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

    let provider = match resolve_tenant_provider(pool, tenant_id).await? {
        Some(p) => p,
        None => env_provider()?,
    };

    dispatch(
        &provider,
        to,
        &final_subject,
        final_body.as_deref(),
        final_html.as_deref(),
    )
    .await
}

/// Fetch a template from the DB and send the email using env-var provider config (system emails
/// with no tenant context). Prefer `send_template_email_for_tenant` when a tenant_id is known.
pub async fn send_template_email(
    pool: &PgPool,
    aid: Uuid,
    to: &str,
    template_type: &str,
    vars: &std::collections::HashMap<&str, &str>,
) -> Result<(), String> {
    let (final_subject, final_body, final_html) =
        render_template(pool, aid, template_type, vars).await;

    let provider = env_provider()?;

    dispatch(
        &provider,
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

// Convenience wrappers for backwards compatibility
pub async fn send_welcome_email(
    pool: &PgPool,
    aid: Uuid,
    to: &str,
    name: &str,
) -> Result<(), String> {
    let mut vars = std::collections::HashMap::new();
    vars.insert("name", name);
    vars.insert("email", to);
    vars.insert("app_url", "https://app.funnelswift.net");
    send_template_email(pool, aid, to, "welcome", &vars).await
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
    vars.insert("app_url", "https://app.funnelswift.net");
    send_template_email(pool, aid, to, "purchase_confirmed", &vars).await
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
    vars.insert("app_url", "https://app.funnelswift.net");
    send_template_email_for_tenant(pool, tenant_id, tenant_id, to, "password_reset", &vars).await
}
