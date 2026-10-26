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

/// Old card spellings that still mean a LIVE catalogue tier (cron 2026-10-25).
///
/// The $79 card shipped `?plan=agency-scale` until 2026-10-23 while the catalogue slug is
/// `agency`. Two edge layers were corrected — the `?plan=` rewrite in `sites/funnelswift.conf` and
/// `PLAN_ALIAS` in `www/funnelswift/signup.html` — but the door that WRITES the plan request did
/// not know the mapping, so any caller arriving without passing through those two (a replayed
/// marketing link aimed at the API host, an integration, a page added later) had its paid choice
/// dropped in silence. Measured live 2026-10-25: `plan=agency-scale` → `plan_request: null`,
/// `plan=agency` → request filed.
///
/// Anything unknown is returned unchanged, so an unrecognised slug still resolves to nothing and
/// the caller gets the warning below instead of a wrong tier.
pub(crate) fn legacy_plan_slug(slug: &str) -> &str {
    match slug {
        "agency-scale" => "agency",
        other => other,
    }
}

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

/// POST /api/v1/auth/signup — the PUBLIC signup, shared by BOTH products: the MAIN FunnelSwift page
/// (`funnelswift.net/signup.html`, `source: 'homepage' | 'affiliate_referral'`) and the Kinetic
/// Cards landing page (`funnelswift.net/kinetic`, `source: 'kinetic-landing'`). Creates tenant +
/// user and seats the FREE tier of the side the request names (capture for the main product,
/// kinetic for the landing page). A `plan` slug in the body is never trusted as the plan.
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
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let Json(payload) = body.map_err(crate::error::json_rejection)?;
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
    // ── VALIDATE THE WRITE ─────────────────────────────────────────────────────────────────────
    // `tenants.name` is derived as `"<name>'s Workspace"` and `tenants.slug` as `"<name>-<8 hex>"`,
    // and both columns are `varchar(255)` — so a 254-character name made the FIRST statement of the
    // signup transaction the one the database refused. Measured live 2026-10-16: 200 characters
    // created the account, 254 answered `500 {"error":"Database error"}` with no account and no hint
    // that the name was the cause (the form carries no `maxlength`, so a pasted paragraph is an
    // ordinary way to hit it). `name` is rebound here, so every later use — the workspace row, its
    // slug, `users.name`, the credentials mail and the response — carries the validated value. Too
    // long is a 422 and nothing is written.
    let name =
        crate::security::display_name::normalize(&name).map_err(AppError::UnprocessableEntity)?;
    // ── OPTIONAL WORKSPACE NAME (2026-10-18) ────────────────────────────────────────────────────
    // A client that also collects a business name (the mobile app's signup) may send `tenant_name`;
    // this door is the fleet's name+email-only door, so it must honour that field or the business
    // name a customer typed is silently thrown away. Absent or blank keeps the historical
    // derivation, so the web form's behaviour is byte-identical. Validated the same way `name` is,
    // because it lands in the same two `varchar(255)` columns.
    let workspace_name = payload["tenant_name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{name}'s Workspace"));
    crate::security::display_name::check_len(
        &workspace_name,
        crate::security::display_name::MAX_NAME_CHARS,
        "tenant_name",
    )
    .map_err(AppError::UnprocessableEntity)?;

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
        crate::security::display_name::slugify(&workspace_name),
        Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    // Mark the tenant as harness-created: the `X-Swift-Harness` header when the caller sent one,
    // otherwise the recipient's own harness/reserved domain (`probe_harness::for_signup`, measured
    // 2026-10-17). The marker is NEVER read from `payload` — a body field would let a customer mark
    // their own tenant. A real customer's domain is on neither list => NULL, byte-identical row.
    let probe_harness = crate::probe_harness::for_signup(&headers, &email);
    // ONE transaction (measured defect, 2026-10-12 — the same class the `register` handler carried).
    // The workspace, its owner row and its default settings used to be three separate INSERTs on
    // three pooled connections: a signup that failed after the tenant INSERT left the workspace
    // behind while the caller got a 500, and repeated attempts filled the operator's tenant list
    // with empty "…'s Workspace" rows. Committing only once the account is complete means a failed
    // signup writes nothing at all.
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO tenants (id, name, slug, probe_harness) VALUES ($1, $2, $3, $4)")
        .bind(tenant_id)
        .bind(&workspace_name)
        .bind(&tenant_slug)
        .bind(&probe_harness)
        .execute(&mut *tx)
        .await?;

    // Create user
    // `users_email_key` is the real guard: the duplicate SELECT above is only a fast path, so two
    // racing signups for one address both pass it. The loser must get the same 409 the pre-check
    // returns, not the 500 a raw `?` turned the unique violation into.
    let user_id = Uuid::new_v4();
    let user_insert = sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .bind("user")
    .execute(&mut *tx)
    .await;
    if let Err(e) = user_insert {
        tx.rollback().await.ok();
        if let sqlx::Error::Database(db) = &e {
            if db.constraint() == Some("users_email_key") {
                return Err(AppError::Conflict(
                    "An account with this email already exists".into(),
                ));
            }
        }
        return Err(AppError::Database(e));
    }

    // Create default lead stages
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(json!(["New", "Contacted", "Qualified", "Proposal", "Negotiation", "Closed Won", "Closed Lost"]))
    .execute(&mut *tx)
    .await?;

    // The workspace, its owner and its settings now exist as ONE unit. Commit BEFORE the plan
    // assignment and the credentials mail — both are published outcomes, not account rows.
    tx.commit().await?;

    // Assign the free tier of THIS product (kanban t_3641f326).
    //
    // This used to be `let plan_slug = "kinetic-free"` + `SELECT id FROM plans WHERE slug = $1`
    // and `if let Some(pid) { … }` with no else arm, so a from-zero install (generic plans:
    // free/starter/pro/enterprise) completed the signup with NO `tenant_plan_subscriptions` row,
    // silently. `plans.price` and `plans.side` are both NOT NULL, so the resolver always finds
    // the install's free tier, and it reports which one: a WARN when this install has no free plan
    // for the kinetic side, an ERROR plus `plan_assignment.warning` when it has none at all.
    // WHICH SIDE is derived from the REQUEST — never hardcoded (measured and fixed 2026-10-12).
    //
    // `POST /api/v1/auth/signup` is the public signup for BOTH products: the Kinetic Cards landing
    // page (`funnelswift.net/kinetic`, which sends `source: 'kinetic-landing'` and `plan:
    // 'kinetic-free'`) AND the MAIN FunnelSwift signup page (`funnelswift.net/signup.html`, which
    // sends `source: 'homepage'` or `'affiliate_referral'` and `plan: 'free'`). Pinning the Kinetic
    // side here seated every homepage and affiliate signup on `kinetic-free`, whose `max_forms` and
    // `max_leads` are both 0 — so a customer who signed up for the funnel / lead-capture product
    // landed on a plan with ZERO leads and ZERO forms (the web-to-lead builder answered 402; the
    // affiliate-only carve-out in `web_to_lead_handler` hid the rest). The side now follows the
    // `source` marker the two pages already send, defaulting to the MAIN product this route's own
    // page serves.
    let side = if source == "kinetic-landing"
        || payload["plan"]
            .as_str()
            .map(|p| p.starts_with("kinetic"))
            .unwrap_or(false)
    {
        crate::plan_resolver::ProductSide::Kinetic
    } else {
        crate::plan_resolver::ProductSide::Capture
    };
    let plan = crate::plan_resolver::assign_free_plan(
        &state.pool,
        tenant_id,
        side,
        "POST /api/v1/auth/signup",
    )
    .await?;

    // ── THE PLAN THE VISITOR ACTUALLY CHOSE (2026-10-21) ────────────────────────────────────────
    // A visitor who picks a PAID card on /pricing is seated on that side's FREE tier (there is no
    // payment provider yet), and until now their choice was dropped on the floor: the signup page
    // said "you can move up any time from your dashboard", the dashboard showed Free, and the
    // operator never learned that a hot lead had asked for Pro. Record the ask as a PLAN REQUEST —
    // the same row the dashboard's own Upgrade button files — so it reaches the operator's queue
    // and the customer's own "your request is with us" card immediately.
    //
    // Only when the slug names a PAID plan on THIS workspace's side: a missing/unknown slug, the
    // free tier itself, or a slug belonging to the other product writes nothing. `price > 0` is the
    // paid test (both columns are NOT NULL, the same predicate `plan_resolver` uses), and the insert
    // is guarded by "no open ask for this workspace and plan" so a double-submit cannot duplicate.
    // Harness signups ARE recorded — the operator's QUEUE hides harness tenants instead
    // (`plan_request_handler::admin_list`, `?include_probe=1`), so attribution stays in the data
    // and the list the operator reads stays clean.
    let requested_slug = payload["plan"]
        .as_str()
        .map(|p| p.trim().to_lowercase())
        .unwrap_or_default();
    // ── A LEGACY SPELLING STILL NAMES ITS TIER (cron 2026-10-25) ────────────────────────────────
    // The $79 card shipped `?plan=agency-scale` until 2026-10-23 while the catalogue's slug is
    // `agency`. The correction lives in TWO edge layers — the `?plan=` rewrite in
    // `sites/funnelswift.conf` and `PLAN_ALIAS` in `www/funnelswift/signup.html` — and nothing at
    // the layer that actually owns plan resolution, so a caller that reaches THIS door directly
    // (a replayed marketing link against the API host, an integration, a future page) still lost
    // the ask. Measured live 2026-10-25 before this: `POST /api/v1/auth/signup` with
    // `plan=agency-scale` answered `plan_request: null` and wrote no row, while `plan=agency`
    // filed the request. Same request, same door, one spelling.
    // Keep this map in sync with those two edge copies.
    let requested_slug = legacy_plan_slug(&requested_slug).to_string();
    let mut plan_request: Option<Value> = None;
    if !requested_slug.is_empty() {
        let side_word = match side {
            crate::plan_resolver::ProductSide::Capture => "main",
            crate::plan_resolver::ProductSide::Kinetic => "kinetic",
        };
        let target: Option<(String, String)> = sqlx::query_as(
            "SELECT slug, name FROM plans
              WHERE slug = $1 AND price > 0 AND side IN ($2, 'both')",
        )
        .bind(&requested_slug)
        .bind(side_word)
        .fetch_optional(&state.pool)
        .await?;
        if let Some((slug, plan_name)) = target {
            let row = sqlx::query(
                "INSERT INTO plan_requests (tenant_id, user_id, requested_by, plan_slug, plan_name, note)
                 SELECT $1, $2, $3, $4, $5, $6
                  WHERE NOT EXISTS (
                        SELECT 1 FROM plan_requests
                         WHERE tenant_id = $1 AND plan_slug = $4 AND status IN ('open','contacted'))
                 RETURNING id, status, created_at::text AS created_at",
            )
            .bind(tenant_id)
            .bind(user_id)
            .bind(&email)
            .bind(&slug)
            .bind(&plan_name)
            .bind("Chosen on the pricing page at signup. Checkout has no payment provider yet, so the workspace starts on the free tier of the same product.")
            .fetch_optional(&state.pool)
            .await?;
            if let Some(r) = row {
                tracing::info!(
                    email = %email,
                    plan = %slug,
                    "signup: the paid plan this visitor chose is recorded as a plan request"
                );
                plan_request = Some(json!({
                    "id": r.get::<Uuid, _>("id"),
                    "plan_slug": slug,
                    "plan_name": plan_name,
                    "status": r.get::<String, _>("status"),
                    "created_at": r.get::<String, _>("created_at"),
                }));
            }
        } else {
            // A non-empty slug that names no live PAID tier on this side used to vanish with no
            // trace at all — the visitor saw "you can move up any time", the operator's queue
            // stayed empty, and nothing in the log said why. One line, naming the exact word the
            // caller sent, is what makes the next occurrence diagnosable instead of invisible.
            tracing::warn!(
                email = %email,
                plan = %requested_slug,
                side = side_word,
                "signup: the tier this visitor chose names no live paid plan on this side, so no \
                 plan request was recorded"
            );
        }
    }

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
    // ── WHY ARM 1 READS THE HEADER, NOT THE TENANT MARKER (measured live 2026-10-24) ───────────
    // `probe_harness` is now ALSO set for a disposable-inbox address (`maxxspace.com`, via
    // `probe_addr::attribution_domain`) so a probe workspace no longer appears in the operator's
    // tenant list as a customer. That is ATTRIBUTION, and it must not silence the mail: the
    // credential-delivery proof reads the generated password back out of that inbox, so the moment
    // this arm keyed on the marker the browser signup was recorded `withheld` instead of sending
    // (`email_send_log`: "signup credentials email (withheld)", 2026-10-24 22:53Z). Arm 1 therefore
    // asks the REQUEST whether it declared itself a harness (`X-Swift-Harness`), which is the
    // suppression decision; the derived marker stays attribution-only. Arm 2 is unchanged: a
    // fleet-harness DOMAIN still suppresses (its mail would reach nobody).
    let probe_send_reason = crate::probe_harness::from_headers(&headers)
        .map(|marker| format!("X-Swift-Harness: {marker}"))
        .or_else(|| {
            crate::security::probe_addr::harness_domain(&email)
                .map(|domain| format!("fleet harness domain {domain}"))
        });
    match probe_send_reason {
        Some(reason) => {
            tracing::info!(
                email = %email,
                reason = %reason,
                "signup: credentials email SUPPRESSED — probe/harness account. The account is real and \
                 the generated password was returned to the caller; no mail was sent."
            );
            // The credentials mail is the one a new customer needs first, so a withheld one must be
            // visible on the operator's line instead of leaving an older "accepted" send standing.
            crate::email::record_withheld(&state.pool, "signup credentials email", &email, &reason)
                .await;
        }
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
            "plan_side": side.label(),
            "plan_assignment": plan.to_json(),
            "plan_request": plan_request,
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

#[cfg(test)]
mod legacy_plan_slug_tests {
    use super::legacy_plan_slug;

    /// The spelling the $79 card shipped until 2026-10-23 must name the tier that card sold.
    #[test]
    fn the_retired_agency_spelling_still_names_agency() {
        assert_eq!(legacy_plan_slug("agency-scale"), "agency");
    }

    /// Every live slug passes through untouched — the alias must not rewrite the catalogue.
    #[test]
    fn live_slugs_are_unchanged() {
        for slug in [
            "capture-free",
            "capture-starter",
            "kinetic-free",
            "kinetic-pro",
            "suite",
            "agency",
        ] {
            assert_eq!(legacy_plan_slug(slug), slug);
        }
    }

    /// An unknown word stays unknown, so the handler's warning fires instead of a wrong tier
    /// being recorded.
    #[test]
    fn unknown_words_are_not_invented_into_a_tier() {
        assert_eq!(legacy_plan_slug("notaplan"), "notaplan");
        assert_eq!(legacy_plan_slug(""), "");
    }
}
