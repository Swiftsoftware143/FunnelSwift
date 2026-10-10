//! Email provider configuration + delivery — **DB ONLY** (no env-var credentials).
//!
//! Nothing here reads the process environment. Credentials come from the database and are
//! entered in the admin panel (Admin > Settings > Email Provider).
//!
//! Resolution order for a tenant:
//!   1. `tenant_settings` key `email_config`   (explicit `provider`: smtp|mailgun|sendgrid|sendiio)
//!   2. `tenant_settings` key `mailgun_config` (legacy row → provider "mailgun")
//!   3. `tenant_settings` key `smtp_config`    (legacy row → provider "smtp")
//!   4. `admin_settings`  key `email`          (global system mail, admin-editable)
//!
//! Unconfigured resolves to `None`; every caller logs and skips (never panics, never
//! silently falls back to a server-wide env var).

use crate::security::provider_key_crypto;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct EmailConfig {
    pub provider: String,
    pub api_url: String,
    pub api_key: String,
    pub domain: String,
    pub from_address: String,
    pub from_name: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_encryption: String,
}

fn s(cfg: &Value, key: &str) -> String {
    cfg.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Accept both the new generic names and the legacy Mailgun/SMTP row names.
fn first(cfg: &Value, keys: &[&str]) -> String {
    for k in keys {
        let v = s(cfg, k);
        if !v.is_empty() {
            return v;
        }
    }
    String::new()
}

impl EmailConfig {
    pub fn from_json(cfg: &Value, default_provider: &str) -> EmailConfig {
        let provider = {
            let p = s(cfg, "provider").to_ascii_lowercase();
            if p.is_empty() {
                default_provider.to_string()
            } else {
                p
            }
        };
        let from_address = first(cfg, &["from_address", "from_email"]);
        let from_name = {
            let n = first(cfg, &["from_name"]);
            if n.is_empty() {
                "FunnelSwift".to_string()
            } else {
                n
            }
        };
        EmailConfig {
            provider,
            api_url: first(cfg, &["api_url", "base_url"]),
            api_key: first(cfg, &["api_key"]),
            domain: first(cfg, &["domain", "mailgun_domain"]),
            from_address,
            from_name,
            smtp_host: first(cfg, &["smtp_host", "host"]),
            smtp_port: cfg
                .get("smtp_port")
                .or_else(|| cfg.get("port"))
                .and_then(|v| v.as_u64())
                .unwrap_or(587) as u16,
            smtp_username: first(cfg, &["smtp_username", "username", "user"]),
            smtp_password: first(cfg, &["smtp_password", "password", "pass"]),
            smtp_encryption: first(cfg, &["smtp_encryption", "encryption"]),
        }
    }

    /// True when the stored row actually carries what its transport needs.
    pub fn is_configured(&self) -> bool {
        match self.provider.as_str() {
            "smtp" => !self.smtp_host.is_empty() && !self.from_address.is_empty(),
            _ => {
                (!self.api_key.is_empty() || !self.api_url.is_empty())
                    && !self.from_address.is_empty()
            }
        }
    }

    pub fn sender(&self) -> String {
        if self.from_name.is_empty() {
            self.from_address.clone()
        } else {
            format!("{} <{}>", self.from_name, self.from_address)
        }
    }
}

/// The credential fields carried inside an `admin_settings.email` (or tenant `email_config`)
/// object. They are sealed with the SAME `enc:v1:` envelope the app already uses for
/// `provider_keys`/`payment_providers` — this config was the one credential path that stored
/// its value in the clear (kanban t_a794cb09), so a database dump or a backup yielded a
/// usable Mailgun private key for the whole fleet.
///
/// THE LIST MUST MATCH WHAT [`EmailConfig::from_json`] READS, not just what the panel writes.
/// `from_json` resolves the SMTP password from `smtp_password` OR the legacy aliases `password` /
/// `pass`, and both settings routes accept an ARBITRARY JSON object, so a two-name vocabulary
/// sealed only the first: a tenant posting `{"email_config":{"password":"…"}}` through
/// `PUT /api/v1/settings` stored a PLAINTEXT credential that `resolve` → `send_via_smtp` then
/// used — leaked AND live (kanban t_b040a78e, the same hole ADASwift closed in t_a8ee62bd).
/// Every seal/open/mask/restore/remover path iterates THIS array, so adding a name here moves all
/// of them in one commit; `migrations/084_tenant_settings_config_secrets_sealed.sql` enforces the
/// same vocabulary in the database and `ensure_tenant_config_seal_guard` re-arms it at boot.
pub const CONFIG_SECRET_FIELDS: [&str; 4] = ["api_key", "smtp_password", "password", "pass"];

/// Seal the credential fields of an email-config object IN PLACE, before it is stored.
///
/// * empty stays empty — a blank field is "no credential", never a ciphertext of nothing;
/// * an already-sealed value is left exactly as it is: that is what the panel's masked
///   round-trip carries back, and re-encrypting it would destroy the stored credential;
/// * a missing master key makes this FAIL — a plaintext credential is never a fallback.
pub async fn seal_config_secrets(
    pool: &PgPool,
    cfg: &mut Value,
) -> Result<(), provider_key_crypto::CryptoError> {
    let Some(obj) = cfg.as_object_mut() else {
        return Ok(());
    };
    for field in CONFIG_SECRET_FIELDS {
        let current = obj
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if current.is_empty() || provider_key_crypto::is_encrypted(&current) {
            continue;
        }
        let sealed = provider_key_crypto::encrypt_for_storage(pool, &current).await?;
        obj.insert(field.to_string(), Value::String(sealed));
    }
    Ok(())
}

/// Open the credential fields of an email-config object IN PLACE after a DB read, so what
/// reaches a provider is the credential and never the envelope. A value without the envelope
/// is a legacy plaintext row and is passed through unchanged.
pub async fn open_config_secrets(
    pool: &PgPool,
    cfg: &mut Value,
) -> Result<(), provider_key_crypto::CryptoError> {
    let Some(obj) = cfg.as_object_mut() else {
        return Ok(());
    };
    for field in CONFIG_SECRET_FIELDS {
        let current = obj
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if current.is_empty() || !provider_key_crypto::is_encrypted(&current) {
            continue;
        }
        let opened = provider_key_crypto::decrypt_from_storage(pool, &current).await?;
        obj.insert(field.to_string(), Value::String(opened));
    }
    Ok(())
}

/// Seal every credential still sitting in the clear in the `admin_settings.email` row.
///
/// Both write paths seal before they store, but this row can also arrive plaintext from a
/// database restored out of a dump taken before the change, or from a writer added later that
/// forgets. Idempotent; returns the number of rows it had to rewrite.
pub async fn seal_legacy_config_secrets(
    pool: &PgPool,
) -> Result<u64, provider_key_crypto::CryptoError> {
    let mut value: Option<Value> =
        sqlx::query_scalar("SELECT value FROM admin_settings WHERE key = 'email'")
            .fetch_optional(pool)
            .await?;
    let Some(mut value) = value.take() else {
        return Ok(0);
    };
    if !value.is_object() {
        return Ok(0);
    }
    let before = value.clone();
    seal_config_secrets(pool, &mut value).await?;
    if value == before {
        return Ok(0);
    }
    sqlx::query(
        "UPDATE admin_settings SET value = $1::jsonb, updated_at = NOW() WHERE key = 'email'",
    )
    .bind(&value)
    .execute(pool)
    .await?;
    Ok(1)
}

/// The one string a credential is replaced with in a response, on EVERY surface that renders one
/// (the tenant settings view and the admin email-config panel). It is also the marker that means
/// "keep the stored credential" when it comes back on a save.
pub const SECRET_MASK: &str = "••••••••";

/// True when an incoming credential value is really the MASK (or blank, which the panels use for
/// "leave the stored one alone") rather than a new credential. One predicate for every surface.
pub fn is_masked(v: &str) -> bool {
    v.is_empty() || v.chars().all(|c| c == '•' || c == '*')
}

/// The three TENANT-side mail-config keys `resolve` reads (resolution order 1-3) — the only keys
/// whose `value` may carry a provider credential. A tenant's own settings route
/// (`PUT /api/v1/settings`) takes an ARBITRARY `{key, value}` pair, so these are exactly the keys
/// that must be sealed on write and masked on read (kanban t_b040a78e).
pub const TENANT_CONFIG_KEYS: [&str; 3] = ["email_config", "mailgun_config", "smtp_config"];

/// Seal every credential still sitting in the clear in a TENANT's own mail-config rows
/// (`tenant_settings` keys [`TENANT_CONFIG_KEYS`]).
///
/// The tenant writer seals before it stores, exactly as the admin writer does, but these rows can
/// also arrive plaintext from a database restored out of an older dump — or from a writer added
/// later that forgets. This is the boot half that converges them. Idempotent; returns the number
/// of rows it had to rewrite.
pub async fn seal_legacy_tenant_config_secrets(
    pool: &PgPool,
) -> Result<u64, provider_key_crypto::CryptoError> {
    let mut sealed = 0u64;
    for key in TENANT_CONFIG_KEYS {
        let rows: Vec<(Uuid, Value)> =
            sqlx::query_as("SELECT tenant_id, value FROM tenant_settings WHERE key = $1")
                .bind(key)
                .fetch_all(pool)
                .await?;
        for (tenant_id, mut value) in rows {
            if !value.is_object() {
                continue;
            }
            let before = value.clone();
            seal_config_secrets(pool, &mut value).await?;
            if value == before {
                continue;
            }
            sqlx::query(
                "UPDATE tenant_settings SET value = $1::jsonb, updated_at = NOW()
                  WHERE tenant_id = $2 AND key = $3",
            )
            .bind(&value)
            .bind(tenant_id)
            .bind(key)
            .execute(pool)
            .await?;
            sealed += 1;
        }
    }
    Ok(sealed)
}

/// Open + MASK the credential fields of ONE tenant mail-config object for the tenant's own view.
///
/// The mask is computed from the OPENED value, never from the stored bytes: a mask derived from
/// the ciphertext (`enc...XYZ`) is itself the defect, and a row this deployment cannot open must
/// be reported as NOT SET rather than shipping the envelope to the client. `open_config_secrets`
/// is what makes that distinction — a read path that merely always masks reports every row as set.
///
/// Fields the row does not carry are left alone, and non-secret fields (`provider`, `from_address`,
/// …) pass through untouched so the panel round-trips.
pub async fn open_and_mask_config_secrets(pool: &PgPool, value: &Value) -> Value {
    if !value.is_object() {
        return value.clone();
    }
    let mut opened = value.clone();
    let openable = open_config_secrets(pool, &mut opened).await.is_ok();
    let mut out = value.clone();
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    for field in CONFIG_SECRET_FIELDS {
        if !obj.contains_key(field) {
            continue;
        }
        let plain = if openable {
            opened.get(field).and_then(|v| v.as_str()).unwrap_or("")
        } else {
            ""
        };
        let set = !plain.is_empty();
        obj.insert(
            field.to_string(),
            Value::String(if set {
                SECRET_MASK.to_string()
            } else {
                String::new()
            }),
        );
        obj.insert(format!("{}_set", field), Value::Bool(set));
    }
    out
}

/// Restore the STORED credential for every field the caller sent back MASKED, and strip the
/// `<field>_set` markers the view answers with (they are UI scaffolding, not config).
///
/// This is what makes the masked round-trip safe: without it the literal mask is stored as the
/// provider credential. The stored value is the sealed ciphertext, so it is copied verbatim —
/// `seal_config_secrets` skips an already-sealed value rather than re-encrypting it.
pub fn restore_masked_config_secrets(incoming: &mut Value, stored: Option<&Value>) {
    let Some(obj) = incoming.as_object_mut() else {
        return;
    };
    for field in CONFIG_SECRET_FIELDS {
        let sent = obj
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if is_masked(&sent) {
            let kept = stored
                .and_then(|s| s.get(field))
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            obj.insert(field.to_string(), kept);
        }
        obj.remove(&format!("{}_set", field));
    }
}

/// The database-level regression guard on the tenant mail-config rows (kanban t_b040a78e item 2):
/// a plaintext credential in any of [`CONFIG_SECRET_FIELDS`] under one of [`TENANT_CONFIG_KEYS`]
/// is refused by the store, whatever writer forgot to seal it.
pub const TENANT_CONFIG_SEAL_CONSTRAINT: &str = "tenant_settings_config_secrets_sealed";

/// The guard's predicate, in ONE place: `migrations/084_…` installs exactly this expression and the
/// boot half below re-arms it with the same text, so the two can never disagree.
///
/// NOTE: these are COMPILE-TIME LITERALS on purpose — the fleet gate refuses a statement built at
/// run time (gate rule 5d, class 14), so this is one literal per statement rather than a `format!`.
/// The constraint NAME is therefore repeated across the literals; the unit tests at the bottom of
/// this file assert that every one of them names the same constraint and covers every one of
/// `CONFIG_SECRET_FIELDS` and `TENANT_CONFIG_KEYS`, so the repetition cannot drift silently.
const TENANT_CONFIG_SEAL_EXISTS_SQL: &str = "\
SELECT EXISTS (SELECT 1 FROM pg_constraint \
 WHERE conname = 'tenant_settings_config_secrets_sealed' \
   AND conrelid = 'public.tenant_settings'::regclass)";

const TENANT_CONFIG_SEAL_ADD_SQL: &str = "\
ALTER TABLE public.tenant_settings ADD CONSTRAINT tenant_settings_config_secrets_sealed \
CHECK (key NOT IN ('email_config','mailgun_config','smtp_config') \
OR ( \
(coalesce(value->>'api_key','') = '' OR value->>'api_key' LIKE 'enc:v1:%') \
AND (coalesce(value->>'smtp_password','') = '' OR value->>'smtp_password' LIKE 'enc:v1:%') \
AND (coalesce(value->>'password','') = '' OR value->>'password' LIKE 'enc:v1:%') \
AND (coalesce(value->>'pass','') = '' OR value->>'pass' LIKE 'enc:v1:%'))) NOT VALID";

const TENANT_CONFIG_SEAL_VALIDATED_SQL: &str = "\
SELECT convalidated FROM pg_constraint \
 WHERE conname = 'tenant_settings_config_secrets_sealed' \
   AND conrelid = 'public.tenant_settings'::regclass";

const TENANT_CONFIG_SEAL_VALIDATE_SQL: &str = "\
ALTER TABLE public.tenant_settings VALIDATE CONSTRAINT tenant_settings_config_secrets_sealed";

/// How many covered rows still hold an unsealed credential — the count that decides whether the
/// guard can be validated, and the one the boot log prints.
const TENANT_CONFIG_UNSEALED_SQL: &str = "\
SELECT count(*) FROM tenant_settings \
 WHERE key IN ('email_config','mailgun_config','smtp_config') \
   AND jsonb_typeof(value) = 'object' \
   AND ((coalesce(value->>'api_key','') <> '' AND value->>'api_key' NOT LIKE 'enc:v1:%') \
     OR (coalesce(value->>'smtp_password','') <> '' AND value->>'smtp_password' NOT LIKE 'enc:v1:%') \
     OR (coalesce(value->>'password','') <> '' AND value->>'password' NOT LIKE 'enc:v1:%') \
     OR (coalesce(value->>'pass','') <> '' AND value->>'pass' NOT LIKE 'enc:v1:%'))";

/// Boot half of the DB guard: **add-if-missing, then validate-or-warn**.
///
/// `migrations/084_…` installs the constraint once (the ledger records the file and never re-runs
/// it), so a constraint dropped by hand, lost in a partial restore or missing because the table
/// pre-existed would otherwise stay absent forever. This is the repair path: every boot re-adds it
/// when it is gone, and VALIDATEs it as soon as no unsealed row remains (which is also what
/// re-claims `t` after the boot seal has converged a restored dump). A boot that changes nothing
/// logs nothing.
///
/// Never fatal: a broken credential row must not stop the app booting. Call it AFTER
/// [`seal_legacy_tenant_config_secrets`], so the seal has already converged the rows this counts.
pub async fn ensure_tenant_config_seal_guard(pool: &PgPool) {
    let present: Result<bool, sqlx::Error> = sqlx::query_scalar(TENANT_CONFIG_SEAL_EXISTS_SQL)
        .fetch_one(pool)
        .await;

    match present {
        Ok(true) => {}
        Ok(false) => match sqlx::query(TENANT_CONFIG_SEAL_ADD_SQL).execute(pool).await {
            Ok(_) => tracing::warn!(
                "tenant_settings config-seal guard: ADDED (was absent — a hand-dropped or \
                 restored-away constraint has no repair path in the migration ledger)"
            ),
            Err(e) => {
                tracing::error!(
                    "tenant_settings config-seal guard could not be added: {}",
                    e
                );
                return;
            }
        },
        Err(e) => {
            tracing::error!("tenant_settings config-seal guard could not be read: {}", e);
            return;
        }
    }

    let unsealed: Result<i64, sqlx::Error> = sqlx::query_scalar(TENANT_CONFIG_UNSEALED_SQL)
        .fetch_one(pool)
        .await;
    match unsealed {
        Ok(0) => {
            let validated: Result<bool, sqlx::Error> =
                sqlx::query_scalar(TENANT_CONFIG_SEAL_VALIDATED_SQL)
                    .fetch_one(pool)
                    .await;
            match validated {
                // Already fully enforced: nothing to say. The boot log reports changes and
                // posture, not an unchanged state repeated on every start.
                Ok(true) => {}
                Ok(false) => {
                    match sqlx::query(TENANT_CONFIG_SEAL_VALIDATE_SQL)
                        .execute(pool)
                        .await
                    {
                        Ok(_) => tracing::info!(
                            "tenant_settings config-seal guard: VALIDATED (every stored tenant mail \
                             credential is enc:v1: at rest)"
                        ),
                        Err(e) => tracing::warn!(
                            "tenant_settings config-seal guard not validated: {} (new writes are \
                             still checked)",
                            e
                        ),
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "tenant_settings config-seal guard validity read failed: {}",
                        e
                    )
                }
            }
        }
        Ok(n) => tracing::warn!(
            rows = n,
            "tenant_settings config-seal guard: STILL NOT VALID — {} covered row(s) hold an \
             unsealed credential (new writes are rejected, the existing rows are converged by the \
             boot seal)",
            n
        ),
        Err(e) => tracing::error!("tenant_settings config-seal guard census failed: {}", e),
    }
}

async fn row(pool: &PgPool, sql: &str, binds: &[&str]) -> Option<Value> {
    let mut q = sqlx::query_scalar::<_, Value>(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.fetch_optional(pool).await.ok().flatten()
}

/// Resolve the email configuration for a tenant, falling back to the global
/// `admin_settings.email` row (system mail). Returns `None` when nothing is configured.
pub async fn resolve(pool: &PgPool, tenant_id: Option<Uuid>) -> Option<EmailConfig> {
    if let Some(tid) = tenant_id {
        let candidates: [(&str, &str); 3] = [
            ("email_config", "smtp"),
            ("mailgun_config", "mailgun"),
            ("smtp_config", "smtp"),
        ];
        for (key, default_provider) in candidates {
            if let Some(mut v) = row(
                pool,
                "SELECT value FROM tenant_settings WHERE tenant_id = $1 AND key = $2",
                &[&tid.to_string(), key],
            )
            .await
            {
                // The credential is ciphertext at rest; the provider must see the plaintext
                // (an envelope sent as a password is a guaranteed 401, not a send).
                if let Err(e) = open_config_secrets(pool, &mut v).await {
                    tracing::error!(error = %e, key, "tenant email config credential cannot be opened — skipped");
                    continue;
                }
                let cfg = EmailConfig::from_json(&v, default_provider);
                if cfg.is_configured() {
                    return Some(cfg);
                }
            }
        }
    }

    if let Some(mut v) = row(
        pool,
        "SELECT value FROM admin_settings WHERE key = 'email'",
        &[],
    )
    .await
    {
        if let Err(e) = open_config_secrets(pool, &mut v).await {
            tracing::error!(
                error = %e,
                "admin_settings.email credential cannot be opened — treating system mail as \
                 unconfigured (PROVIDER_KEY_ENC_SECRET mismatch?)"
            );
            return None;
        }
        let cfg = EmailConfig::from_json(&v, "smtp");
        if cfg.is_configured() {
            return Some(cfg);
        }
    }

    None
}

/// Deliver a message through the configured provider.
pub async fn deliver(
    cfg: &EmailConfig,
    to: &str,
    subject: &str,
    text: &str,
    html: Option<&str>,
) -> Result<(), String> {
    // ── HARNESS / RESERVED RECIPIENTS NEVER REACH A RELAY (kanban t_36b55ed2) ───────────────────
    // The signup handler refuses this class before it gets here, but two admin surfaces call
    // `deliver` DIRECTLY and so bypassed that guard: the admin create-user credentials send and the
    // "Send test email" button. Measured 2026-10-10: with an `@example.invalid` platform-admin
    // address, "Send test email" reached Mailgun and produced a real bounce. A reserved/harness
    // name can only bounce or land in a fleet mailbox, and every such send burns the domain's
    // sending reputation — the same class the sibling apps suppress inside their own email module.
    // The guard lives at the provider chokepoint so a send path added later inherits it. `*.local`
    // / `localhost` stay OPEN on purpose: the content harness points the provider at a local SMTP
    // sink and reads the message off the wire, and `harness_domain` returns None for it.
    if let Some(domain) = crate::security::probe_addr::harness_domain(to) {
        tracing::info!(
            to = %to,
            domain = %domain,
            "email suppressed: recipient is a fleet harness/reserved address (no send attempted)"
        );
        return Ok(());
    }
    match cfg.provider.as_str() {
        "smtp" => {
            let sc = crate::smtp::SmtpConfig {
                host: cfg.smtp_host.clone(),
                port: cfg.smtp_port,
                username: cfg.smtp_username.clone(),
                password: cfg.smtp_password.clone(),
                from_email: cfg.from_address.clone(),
                from_name: Some(cfg.from_name.clone()),
                encryption: if cfg.smtp_encryption.is_empty() {
                    None
                } else {
                    Some(cfg.smtp_encryption.clone())
                },
            };
            crate::smtp::send_via_smtp(&sc, to, subject, text, html).await
        }
        "sendgrid" => send_sendgrid(cfg, to, subject, text, html).await,
        "sendiio" => send_sendiio(cfg, to, subject, text, html).await,
        // "mailgun" — and any unknown value, so a pre-provider row keeps working.
        _ => send_mailgun(cfg, to, subject, text, html).await,
    }
}

async fn send_mailgun(
    cfg: &EmailConfig,
    to: &str,
    subject: &str,
    text: &str,
    html: Option<&str>,
) -> Result<(), String> {
    let url = if !cfg.api_url.is_empty() {
        cfg.api_url.clone()
    } else if !cfg.domain.is_empty() {
        format!("https://api.mailgun.net/v3/{}/messages", cfg.domain)
    } else {
        return Err("Mailgun api_url/domain not configured".to_string());
    };

    let from = cfg.sender();
    let mut params: Vec<(&str, String)> = vec![
        ("from", from),
        ("to", to.to_string()),
        ("subject", subject.to_string()),
        ("text", text.to_string()),
    ];
    if let Some(h) = html.filter(|h| !h.is_empty()) {
        params.push(("html", h.to_string()));
    }

    let resp = reqwest::Client::new()
        .post(&url)
        .basic_auth("api", Some(&cfg.api_key))
        .form(&params)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Mailgun request failed: {}", e))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Mailgun returned {}: {}", status, body))
    }
}

async fn send_sendgrid(
    cfg: &EmailConfig,
    to: &str,
    subject: &str,
    text: &str,
    html: Option<&str>,
) -> Result<(), String> {
    let url = if cfg.api_url.is_empty() {
        "https://api.sendgrid.com/v3/mail/send".to_string()
    } else {
        cfg.api_url.clone()
    };

    let mut content = vec![json!({ "type": "text/plain", "value": text })];
    if let Some(h) = html.filter(|h| !h.is_empty()) {
        content.push(json!({ "type": "text/html", "value": h }));
    }

    let payload = json!({
        "personalizations": [{ "to": [{ "email": to }] }],
        "from": { "email": cfg.from_address, "name": cfg.from_name },
        "subject": subject,
        "content": content,
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("SendGrid request failed: {}", e))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("SendGrid returned {}: {}", status, body))
    }
}

async fn send_sendiio(
    cfg: &EmailConfig,
    to: &str,
    subject: &str,
    text: &str,
    html: Option<&str>,
) -> Result<(), String> {
    let url = if cfg.api_url.is_empty() {
        "https://sendiio.com/api/v1/smtp/send".to_string()
    } else {
        cfg.api_url.clone()
    };

    let payload = json!({
        "api_key": cfg.api_key,
        "from_email": cfg.from_address,
        "from_name": cfg.from_name,
        "to_email": to,
        "subject": subject,
        "text": text,
        "html": html.unwrap_or(""),
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Sendiio request failed: {}", e))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Sendiio returned {}: {}", status, body))
    }
}

/// Providers the admin can pick — served to the admin UI so the dropdown is not
/// hardcoded in the browser either.
pub fn available() -> Vec<Value> {
    vec![
        json!({"value":"smtp","label":"SMTP (any mail server)"}),
        json!({"value":"mailgun","label":"Mailgun"}),
        json!({"value":"sendgrid","label":"SendGrid"}),
        json!({"value":"sendiio","label":"Sendiio"}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(key: &str, value: &str) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("provider".to_string(), json!("smtp"));
        m.insert("from_address".to_string(), json!("a@b.c"));
        m.insert("smtp_host".to_string(), json!("smtp.probe.invalid"));
        m.insert(key.to_string(), json!(value));
        Value::Object(m)
    }

    /// kanban t_b040a78e item 1 — the SEAL vocabulary has to be the vocabulary `from_json` READS.
    ///
    /// `from_json` resolves the SMTP password from `smtp_password` OR the legacy aliases
    /// `password` / `pass`, so a credential posted under an alias reaches `send_via_smtp`; before
    /// this fix the seal list named only `smtp_password`, so that same alias was stored PLAINTEXT
    /// and then used. Both halves are asserted here, per alias, against the real reader:
    /// `from_json` (this crate's only reader of a stored mail config) and `CONFIG_SECRET_FIELDS`
    /// (the list the seal/open/mask/restore paths iterate).
    #[test]
    fn every_smtp_password_alias_is_read_and_sealed() {
        for alias in ["smtp_password", "password", "pass"] {
            let parsed = EmailConfig::from_json(&cfg_with(alias, "SECRET"), "smtp");
            assert_eq!(
                parsed.smtp_password, "SECRET",
                "from_json did not read the `{}` alias — the credential would be ignored at send \
                 time while the seal path treats it as a credential",
                alias
            );
            assert!(
                CONFIG_SECRET_FIELDS.contains(&alias),
                "`{}` is read as the SMTP password but is NOT in CONFIG_SECRET_FIELDS — a config \
                 posted under it would be stored PLAINTEXT (kanban t_b040a78e)",
                alias
            );
        }
        let parsed = EmailConfig::from_json(&cfg_with("api_key", "SECRET"), "smtp");
        assert_eq!(parsed.api_key, "SECRET");
        assert!(CONFIG_SECRET_FIELDS.contains(&"api_key"));
    }

    /// The other direction of the same invariant: the seal list is exactly the reader's vocabulary
    /// (a name sealed but never read would still be a credential in the clear at some future call
    /// site, and a name read but not sealed is the alias defect above).
    #[test]
    fn secret_field_vocabulary_is_exactly_the_alias_set() {
        let mut expected = vec!["api_key", "smtp_password", "password", "pass"];
        expected.sort_unstable();
        let mut got: Vec<&str> = CONFIG_SECRET_FIELDS.to_vec();
        got.sort_unstable();
        assert_eq!(got, expected);
    }

    /// The guard's SQL is four COMPILE-TIME literals (gate rule 5d refuses a statement built at run
    /// time), so the constraint name and the field/key vocabularies repeat inside them. These
    /// assertions are what keep the copies honest: every DDL literal must name the same constraint,
    /// and the statement that defines the guard plus the census that decides whether it can be
    /// validated must cover every reader name and every key `resolve` reads.
    #[test]
    fn guard_sql_literals_agree_with_the_vocabularies() {
        for sql in [
            TENANT_CONFIG_SEAL_EXISTS_SQL,
            TENANT_CONFIG_SEAL_ADD_SQL,
            TENANT_CONFIG_SEAL_VALIDATED_SQL,
            TENANT_CONFIG_SEAL_VALIDATE_SQL,
        ] {
            assert!(
                sql.contains(TENANT_CONFIG_SEAL_CONSTRAINT),
                "DDL literal does not name {}: {}",
                TENANT_CONFIG_SEAL_CONSTRAINT,
                sql
            );
        }
        assert!(TENANT_CONFIG_SEAL_ADD_SQL.contains("NOT VALID"));
        for field in CONFIG_SECRET_FIELDS {
            assert!(
                TENANT_CONFIG_SEAL_ADD_SQL.contains(field),
                "the DB guard does not cover the `{}` field name",
                field
            );
            assert!(
                TENANT_CONFIG_UNSEALED_SQL.contains(field),
                "the unsealed census does not cover the `{}` field name",
                field
            );
        }
        for key in TENANT_CONFIG_KEYS {
            assert!(
                TENANT_CONFIG_SEAL_ADD_SQL.contains(key),
                "the DB guard does not cover the `{}` key",
                key
            );
            assert!(
                TENANT_CONFIG_UNSEALED_SQL.contains(key),
                "the unsealed census does not cover the `{}` key",
                key
            );
        }
    }
}
