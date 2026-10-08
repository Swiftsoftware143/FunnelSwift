use crate::error::{AppError, AppResult};
use crate::state::AppState;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::models::Claims;

/// Does this `?ref=` code exist? (kanban t_8101edb5, arm (b)).
///
/// The chosen namespace is `tenants.affiliate_code` — the code `kinetic_handler` renders into the
/// card badge `?ref=` URL and the code `affiliate_referral_handler` publishes as the share link.
/// The two legacy namespaces (`affiliates.id`, `affiliate_links.tracking_code`) are resolved too,
/// exactly as the reader does, so this verdict can never disagree with the reader's `resolved`.
/// Always returns one row (bare SELECT), NULLs when nothing matches.
const RESOLVE_SQL: &str = "\
SELECT (SELECT t.id   FROM tenants t WHERE t.affiliate_code = $1 ORDER BY t.created_at LIMIT 1) AS tenant_id,
       (SELECT t.name FROM tenants t WHERE t.affiliate_code = $1 ORDER BY t.created_at LIMIT 1) AS tenant_name,
       (SELECT af.name FROM affiliates af
         WHERE af.id = COALESCE((SELECT al.affiliate_id FROM affiliate_links al
                                  WHERE al.tracking_code = $1 LIMIT 1), $1)
         LIMIT 1) AS affiliate_name";

/// POST /api/v1/auth/signup — Public signup (used by Kinetic Cards landing page).
/// Creates tenant + user, assigns plan from request body (defaults to 'free').
/// A first password the user is expected to replace. 16 characters of UUID-derived entropy from the
/// OS RNG — long enough that the emailed value is not the weak link, short enough to retype on a
/// phone. Never logged.
fn generate_initial_password() -> String {
    let mut out = String::with_capacity(16);
    while out.len() < 16 {
        out.push_str(&Uuid::new_v4().simple().to_string());
    }
    out.truncate(16);
    out
}

pub async fn public_signup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let email = payload["email"].as_str().unwrap_or("").trim().to_string();
    let password = payload["password"].as_str().unwrap_or("").to_string();
    let name = payload["name"].as_str().unwrap_or("").trim().to_string();
    // A `plan` in the body is IGNORED — a public signup only ever starts on the free tier, and
    // WHICH free tier is resolved from `plans.price`/`plans.side` at the assignment site below,
    // never from a slug literal (kanban t_3641f326).
    let source = payload["source"].as_str().unwrap_or("").to_string();
    // (b) VALIDATE THE WRITE — decided arm: STILL-RECORD-AND-FLAG (kanban t_8101edb5).
    //
    // A `?ref=` that resolves to nothing must NOT refuse the signup: the code is copied by hand,
    // shared in DMs and printed on cards, so a typo or a dead link is an ordinary event, and
    // refusing the signup to punish a bad link throws away the customer instead of the code.
    // Sanitising is enforced (trim, empty -> none, cap 100 = the width of `tenants.affiliate_code`);
    // the row is then recorded exactly as it always was, and the verdict is FLAGGED in three places
    // that outlive this request: the `referral.resolved` field below, the log line, and the
    // recomputed `resolved:false` on `GET /api/v1/affiliate-referrals`.
    let affiliate_code = payload["affiliate_code"]
        .as_str()
        .map(|s| s.trim().chars().take(100).collect::<String>())
        .filter(|s| !s.is_empty());

    // ── David's signup model (2026-09-29) ────────────────────────────────────────────────────────
    // The marketing form collects NAME + EMAIL only. The system generates the first password and
    // emails it; the user sets their own in their profile once they are in. A caller that still
    // supplies a password (existing API clients) is honoured and validated exactly as before, so
    // this widens the contract rather than replacing it.
    if email.is_empty() || name.is_empty() {
        return Err(AppError::BadRequest("Name and email are required".into()));
    }
    let password = if password.is_empty() {
        generate_initial_password()
    } else {
        if password.len() < 8 {
            return Err(AppError::BadRequest(
                "Password must be at least 8 characters".into(),
            ));
        }
        password
    };
    // ── VALIDATE THE WRITE (kanban t_38017305) ─────────────────────────────────────────────────
    // The address is trim+lowercased and required to be syntactically an address BEFORE the
    // duplicate SELECT below and before anything is stored or emailed. This used to be
    // `!email.contains('@')`, so `bad@`, `@` or `a b@x` became a real login whose credentials mail
    // could never be delivered. `email` is rebound here, so every later use — the dup check, the
    // INSERT, `referral_tracking`, the JWT claims, the credentials mail and the response — carries
    // the normalised value. A non-address is a 422 `{"error":"email: …"}` and nothing is written.
    let email =
        crate::security::email_addr::normalize(&email).map_err(AppError::UnprocessableEntity)?;

    // Check for duplicate email
    // `lower(email)` so a row stored before normalisation existed still counts as the same address.
    let existing =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_one(&state.pool)
            .await?;

    if existing > 0 {
        return Err(AppError::Conflict(
            "An account with this email already exists".into(),
        ));
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();

    // Create tenant
    let tenant_id = Uuid::new_v4();
    let tenant_slug = format!(
        "{}-{}",
        name.to_lowercase().replace(' ', "-"),
        Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    // Mark the tenant as harness-created if the caller sent a well-formed `X-Swift-Harness`
    // header (kanban t_fc88ec2a). The marker is read from the header ONLY — never from `payload`,
    // which is client-controlled data: a body field would let a customer mark their own tenant.
    // No header (a real customer) => NULL, and the row is byte-identical to before.
    let probe_harness = crate::probe_harness::from_headers(&headers);
    sqlx::query("INSERT INTO tenants (id, name, slug, probe_harness) VALUES ($1, $2, $3, $4)")
        .bind(tenant_id)
        .bind(format!("{}'s Workspace", name))
        .bind(&tenant_slug)
        .bind(&probe_harness)
        .execute(&state.pool)
        .await?;

    // Create user
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .bind("user")
    .execute(&state.pool)
    .await?;

    // Create default lead stages
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(json!(["New", "Contacted", "Qualified", "Proposal", "Negotiation", "Closed Won", "Closed Lost"]))
    .execute(&state.pool)
    .await?;

    // Assign the free tier of THIS product (kanban t_3641f326).
    //
    // This used to be `let plan_slug = "kinetic-free"` + `SELECT id FROM plans WHERE slug = $1`
    // and `if let Some(pid) { … }` with no else arm, so a from-zero install (generic plans:
    // free/starter/pro/enterprise) completed the signup with NO `tenant_plan_subscriptions` row,
    // silently. `plans.price` and `plans.side` are both NOT NULL, so the resolver always finds
    // the install's free tier, and it reports which one: a WARN when this install has no free plan
    // for the kinetic side, an ERROR plus `plan_assignment.warning` when it has none at all.
    let plan = crate::plan_resolver::assign_free_plan(
        &state.pool,
        tenant_id,
        crate::plan_resolver::ProductSide::Kinetic,
        "POST /api/v1/auth/signup",
    )
    .await?;

    // ── THE CREDENTIAL EMAIL ────────────────────────────────────────────────────────────────────
    // Sent after the user row and the plan subscription exist, so the login details are true by the
    // time they arrive. A failure here is LOGGED LOUDLY and never silently swallowed: the signup
    // still succeeds (the account is real), but the operator must be able to see that the password
    // never reached the customer — which is exactly why the old flow was invisible: it generated a
    // password and emailed nobody.
    //
    // ── PROBE / HARNESS ACCOUNTS NEVER SEND (kanban t_36b55ed2) ─────────────────────────────────
    // The fleet's probes mint accounts through THIS route, and the fleet's harness domains
    // (`swiftsoftware.dev` / `.net`) are routable: David was receiving the welcome mail of accounts
    // created by machinery, and every probe burned a delivery on the domain's reputation. Two
    // independent reasons suppress the send, so silencing a probe needs no address change anywhere:
    //   1. the request carried `X-Swift-Harness` — the tenant is harness-marked (attribution by data,
    //      the `harness-marker-at-creation` contract); or
    //   2. the recipient is on a fleet harness domain (`crate::security::probe_addr`) — the layer
    //      that also catches an ad-hoc probe that never learned to send the header.
    // The account itself is still created, with its generated password, exactly as before: a probe
    // that needs the token in the response keeps getting it. Only the mail is withheld. A REAL
    // customer (no header, a domain of their own) reaches the send below byte-identically to before.
    let probe_send_reason = probe_harness
        .as_deref()
        .map(|marker| format!("X-Swift-Harness: {marker}"))
        .or_else(|| {
            crate::security::probe_addr::harness_domain(&email)
                .map(|domain| format!("fleet harness domain {domain}"))
        });
    match probe_send_reason {
        Some(reason) => tracing::info!(
            email = %email,
            reason = %reason,
            "signup: credentials email SUPPRESSED — probe/harness account. The account is real and \
             the generated password was returned to the caller; no mail was sent."
        ),
        None => match crate::email::send_credentials_email(
            &state.pool,
            tenant_id,
            &email,
            &name,
            &password,
        )
        .await
        {
            Ok(()) => tracing::info!(email = %email, "signup: login credentials emailed"),
            Err(e) => tracing::error!(
                email = %email,
                error = %e,
                "signup: account created but the CREDENTIAL EMAIL FAILED — the customer has no password. \
                 Check Admin > Settings > Email Provider."
            ),
        },
    }

    // Log source if provided
    if !source.is_empty() {
        tracing::info!(
            source = %source,
            email = %email,
            plan = ?plan.slug(),
            "Public signup completed"
        );
    }

    // Handle affiliate referral code. The code is resolved against the namespace the rest of the
    // app publishes; an unresolved code is recorded anyway and FLAGGED (arm (b), t_8101edb5) — see
    // `RESOLVE_SQL` and the comment where `affiliate_code` is parsed.
    let mut referral: Option<Value> = None;
    if let Some(ref_code) = affiliate_code {
        let row = sqlx::query(RESOLVE_SQL)
            .bind(&ref_code)
            .fetch_one(&state.pool)
            .await?;
        let owner_id: Option<Uuid> = row.try_get("tenant_id")?;
        let owner_name: Option<String> = row.try_get("tenant_name")?;
        let affiliate_name: Option<String> = row.try_get("affiliate_name")?;
        let resolved = owner_id.is_some() || affiliate_name.is_some();

        if !resolved {
            tracing::warn!(
                affiliate_code = %ref_code,
                signup_email = %email,
                "Unresolved affiliate code: signup recorded and flagged, not refused"
            );
        }

        let _ = sqlx::query(
            r#"INSERT INTO referral_tracking (id, referrer_code, referred_email, referred_tenant_id, created_at)
               VALUES ($1, $2, $3, $4, NOW())"#,
        )
        .bind(Uuid::new_v4())
        .bind(&ref_code)
        .bind(&email)
        .bind(tenant_id)
        .execute(&state.pool)
        .await;

        referral = Some(json!({
            "code": ref_code,
            "resolved": resolved,
            "referrer_tenant": owner_id.map(|id| json!({
                "tenant_id": id.to_string(),
                "name": owner_name.clone().unwrap_or_default(),
            })),
            "affiliate_name": affiliate_name,
        }));
    }

    // Mint the same session JWT the main register path returns. Without it the Kinetic
    // landing page had nothing to hand the app: it stored `undefined` as the token and
    // then redirected to /dashboard on the *marketing* host, which is a 404. The app runs
    // on its own origin, so the caller passes this token over as ?token= (exactly what
    // the main landing page's register flow does); the SPA stores it and strips the URL.
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        tenant_id: tenant_id.to_string(),
        email: email.clone(),
        role: "user".into(),
        exp: now + 86400 * 30,
        iat: now,
        aud: Some("funnelswift-api".to_string()),
        iss: Some("funnelswift".to_string()),
        impersonating: None,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Account created successfully",
            "token": token,
            // The flat slug this response always carried (null only when the install has no free
            // plan at all) plus the structured assignment that can never be silent.
            "plan": plan.slug(),
            "plan_assignment": plan.to_json(),
            "referral": referral,
            "user": {
                "id": user_id,
                "email": email,
                "name": name,
                "role": "user",
                "tenant_id": tenant_id
            }
        })),
    ))
}
