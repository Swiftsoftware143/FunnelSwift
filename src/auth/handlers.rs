use crate::auth::middleware::AuthUser;
use crate::auth::models::*;
use crate::email::send_reset_email;
use crate::error::AppError;
use crate::error::AppResult;
use crate::features;
use crate::state::AppState;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

/// Lazily-generated dummy Argon2 hash used to equalize login timing for unknown
/// email addresses (prevents account-enumeration timing attacks).
fn dummy_password_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"funnelswift-timing-equalizer", &salt)
            .map(|h| h.to_string())
            .unwrap_or_default()
    })
}

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RegisterRequest>, axum::extract::rejection::JsonRejection>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let Json(req) = body.map_err(crate::error::json_rejection)?;
    // Check if email already exists
    // Normalise and validate the address BEFORE the first SELECT (kanban t_38017305): the value
    // that is checked for a duplicate is the exact value that gets stored, put into the JWT claims
    // and returned. This writer had NO guard at all, so `{"email":"bad"}` minted a real account
    // whose login was not an address — unreachable by any credentials mail. A non-address is a 422
    // `{"error":"email: …"}` and nothing is written.
    let email = crate::security::email_addr::normalize(&req.email)
        .map_err(AppError::UnprocessableEntity)?;
    // ── VALIDATE THE WRITE: name lengths ────────────────────────────────────────────────────────
    // `users.name` and `tenants.name`/`tenants.slug` are `varchar(255)` and this writer had no
    // length check, so a long name reached the INSERT and came back as a raw
    // `500 {"error":"Database error"}` inside the signup transaction: the account was rolled back
    // and the caller was told nothing useful. Measured live 2026-10-16 on the sibling public door
    // (254 characters = 500, 200 = created); the same class applies here. Refused BEFORE the first
    // write, as a 422 the console can read.
    crate::security::display_name::normalize(&req.name).map_err(AppError::UnprocessableEntity)?;
    // ── VALIDATE THE WRITE: password policy (measured live 2026-10-16) ─────────────────────────
    // This door is in `PUBLIC_ROUTES` (auth/route_policy.rs), so the caller is anonymous and the
    // `password` string is entirely theirs. With no policy it was hashed verbatim: measured live,
    // `POST /api/v1/auth/register` with `"password": ""` answered 201, and `POST /api/v1/auth/login`
    // with that same empty password on the account it had just minted answered 200 — a working
    // account whose login is the empty string. The four sibling apps either mint a temporary
    // password when the field is empty (and mail it) or refuse anything under 6 characters; this
    // door did neither. Apply the fleet rule here, before anything is hashed and before any row.
    // ── THE FLEET RULE: EMPTY MINTS, SUPPLIED IS CHECKED (cron 2026-10-27) ────────────────────
    // The paragraph above has described the fleet rule since 2026-10-16 — "The four sibling apps
    // either mint a temporary password when the field is empty (and mail it) or refuse anything
    // under 6 characters" — and then applied only the second half, so a caller posting the
    // documented NAME + EMAIL shape was refused 422 "Enter your password." while
    // POST /api/v1/auth/signup answered 201 the same second. The empty value is no longer a weak
    // password: it means "mint one", from the SAME generator and the SAME credentials mailer the
    // public door uses, so the two doors cannot drift. An empty-or-absent password can therefore
    // never become an empty-string credential again (the defect this block was written for).
    let minted_password = req.password.trim().is_empty();
    let password = if minted_password {
        crate::handlers::public_signup_handler::generate_initial_password()
    } else {
        req.password.clone()
    };
    if !minted_password && password.len() < 6 {
        return Err(AppError::UnprocessableEntity(
            "Password must be at least 6 characters".to_string(),
        ));
    }
    if let Some(tenant_name) = req.tenant_name.as_deref() {
        crate::security::display_name::check_len(
            tenant_name,
            crate::security::display_name::MAX_NAME_CHARS,
            "tenant_name",
        )
        .map_err(AppError::UnprocessableEntity)?;
    }

    // Check if email already exists. `lower(email)` so a row stored before normalisation existed
    // (mixed case) still counts as the same address.
    let existing =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_one(&state.pool)
            .await?;

    if existing > 0 {
        return Err(AppError::Conflict(
            "A user with this email already exists. Try signing in.".into(),
        ));
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();

    // Create tenant. ONE name drives BOTH columns (measured defect, 2026-10-17): this door defaulted
    // the workspace NAME to "My Workspace" and the SLUG prefix to "default", so every account minted
    // through it — the native app's fallback, an integration, a harness that does not send
    // `tenant_name` — landed in the operator's tenant list as an identical "My Workspace" row under a
    // `default-…` slug, indistinguishable from the next one. The PUBLIC signup
    // (`handlers::public_signup_handler`) has always derived "<name>'s Workspace" from the person's
    // own name; this door now does the same, and `display_name::normalize` above has already refused
    // an empty name, so the fallback only covers a whitespace-only `tenant_name`.
    let tenant_id = Uuid::new_v4();
    let workspace_name = req
        .tenant_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}'s Workspace", req.name.trim()));
    let tenant_slug = format!(
        "{}-{}",
        crate::security::display_name::slugify(&workspace_name),
        Uuid::new_v4().to_string().get(..8).unwrap_or("x")
    );
    // Mark the tenant as harness-created: the `X-Swift-Harness` header when the caller sent one,
    // otherwise the recipient's own harness/reserved domain (`probe_harness::for_signup`, measured
    // 2026-10-17 — see that fn for why the address arm exists). A real customer's domain is on
    // neither list, so this is NULL and the row is byte-identical to what it was before.
    let probe_harness = crate::probe_harness::for_signup(&headers, &email);
    // ONE transaction, begun before the first write and committed only once the account is
    // complete (measured defect, 2026-10-12). Each INSERT used its own pooled connection, so a
    // signup that failed after the tenant INSERT left the workspace behind while the caller got a
    // 500: eight concurrent registers with one address produced seven 500s and SEVEN orphan
    // tenants (zero users), which sit in the operator's tenant list as "My Workspace". A
    // transaction rolls the loser's workspace back, so the failure is a clean 409 and nothing is
    // written.
    let mut tx = state.pool.begin().await?;

    sqlx::query("INSERT INTO tenants (id, name, slug, probe_harness) VALUES ($1, $2, $3, $4)")
        .bind(tenant_id)
        .bind(&workspace_name)
        .bind(&tenant_slug)
        .bind(&probe_harness)
        .execute(&mut *tx)
        .await?;

    // Create user
    // `users_email_key` is the real guard against a duplicate address; the SELECT above is only a
    // fast path and cannot make the check atomic. Two racing signups for one address both pass it,
    // so the loser must be told the truth — the 409 the pre-check already returns — instead of the
    // 500 a raw `?` turned the unique violation into.
    let user_id = Uuid::new_v4();
    let user_insert = sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind("user")
    .execute(&mut *tx)
    .await;
    if let Err(e) = user_insert {
        tx.rollback().await.ok();
        if let sqlx::Error::Database(db) = &e {
            if db.constraint() == Some("users_email_key") {
                return Err(AppError::Conflict(
                    "A user with this email already exists. Try signing in.".into(),
                ));
            }
        }
        return Err(AppError::Database(e));
    }

    // API-key auto-mint was RETIRED here (kanban t_5a3c2d9c, decision t_538505de ARM b).
    // Signup used to hash a random `fs_…` value with Argon2id and INSERT it into `api_keys` as
    // 'Auto-generated'. That key authenticated nothing (key_hash was never SELECTed for comparison),
    // the register response returns {token,user} only and /me returned the 8-char prefix only, so no
    // caller ever received it. The credential surface (routes, handler, model, this mint) is gone,
    // and t_726416be dropped the `api_keys` TABLE itself (migration 090) together with the last
    // reader — the prefix-only `api_key` field this handler used to add to the /me response.

    // Create default settings for tenant
    sqlx::query(
        "INSERT INTO tenant_settings (id, tenant_id, key, value) VALUES ($1, $2, 'lead_stages', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(serde_json::json!(["New", "Contacted", "Qualified", "Proposal", "Negotiation", "Closed Won", "Closed Lost"]))
    .execute(&mut *tx)
    .await?;

    // The workspace, its owner and its settings now exist as ONE unit. Commit before the plan
    // assignment, whose own failure path is already published in the response.
    tx.commit().await?;

    // Auto-assign the free tier of THIS product (kanban t_3641f326).
    //
    // This used to be `let plan_slug = "capture-free"` + `SELECT id FROM plans WHERE slug = $1`
    // and `if let Some(pid) { … }` with no else arm — so on a from-zero install, whose plans are
    // 000001's generic free/starter/pro/enterprise, the lookup returned None and the signup
    // completed with NO `tenant_plan_subscriptions` row at all, silently. The resolver now binds
    // the free tier through `plans.price = 0` + `plans.side` (both NOT NULL) and reports what it
    // did: a warning in the log when this install has no free plan for this side, an ERROR and a
    // `plan_assignment.warning` in the response when it has no free plan at all.
    // WHICH SIDE is derived from the REQUEST — never hardcoded (measured live 2026-10-21).
    //
    // The pricing cards on funnelswift.com carry the slug the visitor picked into /signup.html
    // (`?plan=kinetic-pro`), and that page posts it back here as `plan`. This handler used to
    // ignore it and pin `ProductSide::Capture`, so a visitor who chose Kinetic Pro landed on a
    // Capture workspace whose dashboard reads "Cards not on this plan" — the exact defect the
    // page-side change was meant to remove, silently re-introduced on the server. Same predicate
    // the sibling public door (`handlers::public_signup_handler::public_signup`) already uses: a
    // slug starting `kinetic` is the Kinetic Cards product, anything else (including a missing
    // field) is the Capture product this route's own page serves.
    let side = match req.plan_slug.as_deref() {
        Some(p) if p.trim().to_lowercase().starts_with("kinetic") => {
            crate::plan_resolver::ProductSide::Kinetic
        }
        _ => crate::plan_resolver::ProductSide::Capture,
    };
    let plan = crate::plan_resolver::assign_free_plan(
        &state.pool,
        tenant_id,
        side,
        "POST /api/v1/auth/register",
    )
    .await?;

    // Generate JWT
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

    // A minted password exists nowhere else, so it MUST reach the customer or the account is
    // unreachable. Same two suppression reasons as the public door (an `X-Swift-Harness` request, or
    // a fleet harness domain) — a probe account is still created and still gets its token, only the
    // mail is withheld. `credentials_emailed` tells the caller the truth either way, which is the
    // field the sibling apps' doors already answer (ADASwift 2026-10-24, public door here 2026-10-24).
    let credentials_emailed = if !minted_password {
        None
    } else {
        let suppressed = crate::probe_harness::from_headers(&headers)
            .map(|marker| format!("X-Swift-Harness: {marker}"))
            .or_else(|| {
                crate::security::probe_addr::harness_domain(&email)
                    .map(|domain| format!("fleet harness domain {domain}"))
            });
        Some(match suppressed {
            Some(reason) => {
                tracing::info!(
                    email = %email,
                    reason = %reason,
                    "register: credentials email SUPPRESSED — probe/harness account; the account is real"
                );
                crate::email::record_withheld(
                    &state.pool,
                    "signup credentials email",
                    &email,
                    &reason,
                )
                .await;
                false
            }
            None => match crate::email::send_credentials_email(
                &state.pool,
                tenant_id,
                &email,
                &req.name,
                &password,
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(email = %email, "register: minted login credentials emailed");
                    true
                }
                Err(e) => {
                    tracing::error!(
                        email = %email,
                        error = %e,
                        "register: account created but the CREDENTIAL EMAIL FAILED — the caller has no password"
                    );
                    false
                }
            },
        })
    };

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "token": token,
            // Which plan the new account landed on (kanban t_3641f326): `plan` stays the flat slug
            // this response's shape always carried (null only if the install has NO free plan at
            // all), `plan_assignment` is the structured form that can never be silent.
            "plan": plan.slug(),
            "plan_assignment": plan.to_json(),
            // `true` only when this door MINTED the password (the caller sent none) AND the mail
            // left; `null` when the caller supplied their own password, where no mail is owed.
            "credentials_emailed": credentials_emailed,
            "user": {
                "id": user_id,
                "email": email,
                "name": req.name,
                "role": "user",
                "tenant_id": tenant_id
            }
        })),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<LoginRequest>, axum::extract::rejection::JsonRejection>,
) -> AppResult<Json<serde_json::Value>> {
    let Json(req) = body.map_err(crate::error::json_rejection)?;
    // Brute-force throttle (cron 2026-10-25): before this, 32 consecutive wrong passwords were
    // answered 401 with no limit of any kind — unlimited guessing against a customer sign-in, and
    // every guess costs a full Argon2 verify. The counter is keyed on the CLIENT ADDRESS the
    // fronting proxy reports, never on the account: a per-account lockout would hand an attacker
    // a denial of service against any known customer. See `crate::login_throttle`.
    let throttle_key = crate::login_throttle::client_identity(&headers);
    if let Err(wait) = state.login_throttle.check(&throttle_key) {
        tracing::warn!(
            "sign-in paused for {}: failed-attempt allowance spent ({}s left)",
            throttle_key,
            wait
        );
        return Err(AppError::TooManyAttempts {
            retry_after_secs: wait,
        });
    }

    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE lower(email) = $1 AND is_active = true",
    )
    .bind(crate::security::email_addr::lookup_key(&req.email))
    .fetch_optional(&state.pool)
    .await?;

    // Always run an Argon2 verification (even for unknown emails) so response timing
    // does not reveal whether an account exists.
    let hash_to_check = match &user {
        Some(u) => u.password_hash.clone(),
        None => dummy_password_hash().to_string(),
    };
    // A stored hash that is not a usable PHC string is NOT a credential: no password can match
    // it, so the caller gets the same generic 401 a wrong password gets. It must never become a
    // 500 — that both announces that this row is broken and makes it a probe target. (The dummy
    // hash used for an unknown address is always parseable, so this arm is only ever a corrupt
    // real row.) Same rule WorkflowSwift's login already states for its placeholder rows.
    let failed = |why: &str| {
        state.login_throttle.record_failure(&throttle_key);
        AppError::Unauthorized(why.to_string())
    };
    let parsed_hash =
        PasswordHash::new(&hash_to_check).map_err(|_| failed("Invalid email or password"))?;
    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| failed("Invalid email or password"))?;

    let user = user.ok_or_else(|| failed("Invalid email or password"))?;

    // A real sign-in clears this address's history, so a customer who mistyped a couple of times
    // is never carried toward a pause by attempts that are behind them.
    state.login_throttle.clear(&throttle_key);

    // tenants.status enforcement, part 1 of 4 (kanban t_af890bbf). A retired workspace cannot mint
    // a new session. Checked AFTER the Argon2 verification above — a WRONG password is still the
    // same 401 as always, so this refuses a correct password (403) without turning the endpoint
    // into an account-existence oracle. `role == "admin"` is exempt: see
    // `handlers::tenant_handler::workspace_access_allowed`.
    if !crate::handlers::tenant_handler::workspace_access_allowed(
        &state.pool,
        user.tenant_id,
        &user.role,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            crate::auth::global_auth::WORKSPACE_INACTIVE.into(),
        ));
    }

    // Stamp the sign-in (migration 118). The operator's workspace people table is the only place
    // that can answer "did this customer ever actually get in?" — the credentials mail is the one
    // thing between a self-serve signup and a usable account, and without this stamp a workspace
    // whose welcome mail never arrived looks exactly like a quiet one. Best-effort on purpose: the
    // caller has already proved the password, so a failed stamp must never turn a good login into
    // an error — it only leaves the timestamp as it was.
    if let Err(e) = sqlx::query("UPDATE users SET last_login_at = NOW() WHERE id = $1")
        .bind(user.id)
        .execute(&state.pool)
        .await
    {
        tracing::warn!(user = %user.id, "sign-in not stamped: {e}");
    }

    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user.id.to_string(),
        tenant_id: user.tenant_id.to_string(),
        email: user.email.clone(),
        role: user.role.clone(),
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

    //     let cs_pool = state.db.clone();
    //     let cs_url = state.coreswift_url.clone();
    //     let cs_key = state.internal_sync_key.clone();
    //     let cs_lead_id = user.id;
    //     let cs_tenant_id = user.tenant_id;
    //     tokio::spawn(async move {
    //         crate::handlers::coreswift_push::push_to_coreswift(
    //             &cs_pool, &cs_url, &cs_key, cs_lead_id, cs_tenant_id,
    //         ).await;
    //     });
    //
    Ok(Json(json!({
        "token": token,
        "user": {
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "role": user.role,
            "tenant_id": user.tenant_id
        }
    })))
}

pub async fn me(state: State<AppState>, auth: AuthUser) -> Json<serde_json::Value> {
    // Get user's name, username, company and profile picture.
    //
    // `company` (migration 098) and `avatar_url` are the two fields the profile screen renders and
    // writes. Both live only on `users` — the picture's BYTES are in `user_avatars` and are read by
    // `get_avatar`, never here, so this row stays small.
    let user_info: Option<(
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        // `email` is read from the ROW, not from the JWT claim (cron 2026-10-18). The claim is a
        // snapshot taken at sign-in, so a customer who had just CONFIRMED a change of sign-in
        // address saw the OLD address on their own profile until they signed in again — the one
        // screen they would go to in order to check. The address is the account's identity, so
        // this answer must come from the account.
        "SELECT name, username, company, avatar_url, email FROM users WHERE id::text = $1",
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    // The `api_key` field of this response was REMOVED (kanban t_726416be). It used to carry the
    // 8-char prefix of the caller's 'Auto-generated' `api_keys` row — the last reader of that table
    // once the credential surface went (t_538505de ARM b / t_5a3c2d9c). Migration 090 drops the
    // table, and no shipped surface ever read this field (box-wide grep: www-app, www-admin and
    // FunnelSwift-Mobile have zero references to `/me`'s `api_key`; the mobile client's `api_key`
    // comes from GET /incentiveswift/config, a different endpoint). Leaving the SELECT would have
    // turned every GET /me into a logged 42P01 with a permanently null field.

    // `available_products` was REMOVED here (kanban t_e0f9b86b). It was built as
    //   SELECT (row_to_json(t.*)::jsonb - 'api_key') AS p FROM target_software t ORDER BY t.name
    //   (that table was itself retired by migration 088 — kanban t_0aaf0bc5 — for holding 0 rows
    //   and dispatching nothing)
    // with no tenant filter, so every authenticated caller got EVERY tenant's routing targets
    // (name, webhook_url, portfolio_company_id, events, is_active). The field is deleted rather
    // than tenant-scoped because no shipped surface reads it (box-wide grep: www-app,
    // www-admin and FunnelSwift-Mobile have zero references) and the same rows are already
    // served, correctly scoped, by GET /target-software (WHERE tenant_id = $1). Deleting it
    // removes the leak with no loss of capability; use that endpoint for a tenant's targets.

    // Get current plan subscription
    let current_plan: Option<serde_json::Value> = sqlx::query_as::<_, (serde_json::Value,)>(
        r#"
            SELECT row_to_json(p.*)::jsonb
            FROM plans p
            JOIN tenant_plan_subscriptions tps ON tps.plan_id = p.id
            WHERE tps.tenant_id::text = $1 AND tps.status = 'active'
            ORDER BY tps.start_date DESC
            LIMIT 1
        "#,
    )
    .bind(&auth.tenant_id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None)
    .map(|(r,)| r);

    // The workspace's actual TIER, as one display string. The console used to paint a raw role word
    // ("User"/"Admin") under the account name, which is where the tier belongs (David 2026-10-08):
    // a customer reads "Kinetic Free", not "User". `current_plan` is the plans row (jsonb) of the
    // active subscription.
    //
    // WHAT WAS WRONG (measured LIVE 2026-10-28): a workspace with NO active subscription answered
    // the literal string "Free". This install's catalogue contains no such tier — its price-0 rows
    // are "Capture Free" and "Kinetic Free" — so the console painted a level that exists in no row,
    // and one the gates do not enforce either (`features::enforce_feature_limit` reads "no active
    // plan -> allow"). Measured: 7 of 13 workspaces were in that state. The app must not invent a
    // level: with no subscription the answer is `plan_name: null` plus `plan_assigned: false`, and
    // the console says "No plan assigned" — the same wording the operator's Tenants list already
    // uses for exactly this state. A workspace that really is on a free tier still reads that
    // tier's own catalogue name, so nothing changes for it.
    let plan_name: Option<String> = current_plan
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);
    let plan_assigned = plan_name.is_some();

    Json(json!({
        "user_id": auth.user_id,
        "tenant_id": auth.tenant_id,
        "email": user_info
            .as_ref()
            .and_then(|(_, _, _, _, e)| e.clone())
            .unwrap_or_else(|| auth.email.clone()),
        "name": user_info.as_ref().map(|(n, _, _, _, _)| n.clone()).unwrap_or_default(),
        "username": user_info.as_ref().and_then(|(_, u, _, _, _)| u.clone()).unwrap_or_default(),
        "company": user_info.as_ref().and_then(|(_, _, c, _, _)| c.clone()).unwrap_or_default(),
        "avatar_url": user_info.as_ref().and_then(|(_, _, _, a, _)| a.clone()).unwrap_or_default(),
        "role": auth.role,
        "is_admin": auth.is_admin,
        "impersonating": auth.impersonating,
        "current_plan": current_plan,
        "plan_name": plan_name,
        // false => this workspace holds NO active subscription. The console says "No plan
        // assigned" instead of guessing a tier; the operator's Tenants list prints the same state,
        // and the operator fixes it with POST /api/v1/tenants/:id/plan.
        "plan_assigned": plan_assigned
    }))
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub username: Option<String>,
    /// The Company the profile screen collects. Nothing else in the product writes it (signup is
    /// NAME + EMAIL only), so this is its only writer.
    pub company: Option<String>,
}

pub async fn update_profile(
    state: State<AppState>,
    auth: AuthUser,
    Json(req): Json<UpdateProfileRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // A blank NAME would leave the console header and every "signed in as" line empty, so it is
    // refused rather than stored. Company may legitimately be cleared, so an empty value is written.
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(AppError::BadRequest("Name cannot be empty".into()));
        }
    }

    // …and the storage limits (measured defect, 2026-10-16): `users.name`, `users.username` and
    // `users.company` are all `varchar(255)`, so a long value reached the UPDATE and answered
    // `500 {"error":"Internal server error"}` — the profile screen's own save failing after the user
    // typed. Refused before the write, as a 422 naming the field. Empty stays legal for
    // `username`/`company` (both are clearable), so only the length is checked for those.
    if let Some(name) = req.name.as_deref() {
        crate::security::display_name::normalize(name).map_err(AppError::UnprocessableEntity)?;
    }
    if let Some(username) = req.username.as_deref() {
        crate::security::display_name::check_len(username, 255, "username")
            .map_err(AppError::UnprocessableEntity)?;
    }
    if let Some(company) = req.company.as_deref() {
        crate::security::display_name::check_len(company, 255, "company")
            .map_err(AppError::UnprocessableEntity)?;
    }

    sqlx::query("UPDATE users SET name = COALESCE($1, name), username = COALESCE($2, username), company = COALESCE($3, company), updated_at = NOW() WHERE id::text = $4")
        .bind(req.name.as_deref().map(str::trim))
        .bind(req.username.as_deref().map(str::trim))
        .bind(req.company.as_deref().map(str::trim))
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update profile: {e}")))?;

    Ok(Json(json!({"status": "ok"})))
}

// The image capability itself — the 2 MB cap, the magic-byte sniff, the multipart read and the
// stored-bytes response — lives in `crate::image_store` (kanban t_c06a32eb), the ONE
// accept/store/serve path, shared with the tenant email logo. This handler keeps only what is
// specific to a PROFILE PICTURE: whose picture it is, which table holds it, and the version-stamped
// URL written onto `users.avatar_url`.

/// `POST /api/v1/auth/avatar` — store the caller's own profile picture and return its URL.
///
/// Storage decision (kanban t_ff948669). The fleet's existing upload shape — IncentiveSwift's
/// `upload_file` — writes the file into a served webroot and stores the resulting URL. This app is
/// a bind-mounted container that exposes ONLY its release binary and `migrations/`: a file written
/// at run time lives inside the container and dies on the next `docker restart`, and no host webroot
/// could serve it either. The image is therefore held in `user_avatars` (one row per user, migration
/// 098) and streamed back by [`get_avatar`]. The multipart read, the size cap and the magic-byte
/// sniff are `crate::image_store`'s, shared with the tenant email logo (kanban t_c06a32eb).
pub async fn upload_avatar(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    request: Request,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user ID".into()))?;

    // The ONE shared image path (crate::image_store): a multipart form OR a raw image body, the
    // 2 MB cap, and the magic-byte sniff to PNG / JPEG / GIF / WebP.
    let (content_type, bytes) =
        crate::image_store::read_request_image(&state, &headers, request).await?;

    sqlx::query(
        r#"INSERT INTO user_avatars (user_id, content_type, bytes, updated_at)
           VALUES ($1, $2, $3, NOW())
           ON CONFLICT (user_id) DO UPDATE
             SET content_type = EXCLUDED.content_type,
                 bytes = EXCLUDED.bytes,
                 updated_at = NOW()"#,
    )
    .bind(user_id)
    .bind(content_type)
    .bind(&bytes)
    .execute(&state.pool)
    .await?;

    // A version stamp in the URL: the bytes behind an avatar change while the path stays the same,
    // and both the browser and the CDN cache by URL.
    let avatar_url = format!("/api/v1/auth/avatar/{user_id}?v={}", Utc::now().timestamp());
    sqlx::query("UPDATE users SET avatar_url = $1, updated_at = NOW() WHERE id = $2")
        .bind(&avatar_url)
        .bind(user_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({ "status": "ok", "avatar_url": avatar_url })))
}

/// `GET /api/v1/auth/avatar/:user_id` — stream a stored profile picture back.
///
/// Deliberately reachable with NO credential (see `auth::route_policy::PUBLIC_ROUTES`): an HTML
/// `<img src>` cannot carry the bearer token, and a profile picture is not a secret. The route can
/// only ever return the image one user uploaded — the id is an unguessable uuid, the stored
/// `content_type` was sniffed from the bytes at upload time, and a user with no picture answers 404
/// so the console falls back to its initial-letter placeholder.
pub async fn get_avatar(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Response> {
    let uid = Uuid::parse_str(&user_id)
        .map_err(|_| AppError::NotFound("No such profile picture".into()))?;

    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT content_type, bytes FROM user_avatars WHERE user_id = $1")
            .bind(uid)
            .fetch_optional(&state.pool)
            .await?;

    let (content_type, bytes) =
        row.ok_or_else(|| AppError::NotFound("No such profile picture".into()))?;

    // The shared response arm (crate::image_store): the sniffed type pinned, `nosniff`, and
    // caching that never invites a shared cache to keep one customer's picture.
    crate::image_store::image_response(content_type, bytes)
}

/// `DELETE /api/v1/auth/avatar` — remove the caller's OWN profile picture.
///
/// The upload twin only ever REPLACES the row (`ON CONFLICT (user_id) DO UPDATE`), so before this
/// route existed a picture could be swapped for another one but never taken down: an operator who
/// wanted their initials back had no way to ask, and the console could only offer "Upload picture".
/// The email-branding logo has had its own DELETE since it shipped, so this closes the same gap on
/// the profile.
///
/// It writes the SESSION's own user — there is no path id to spoof, the same rule `upload_avatar`
/// follows — and it is idempotent: `removed` reports whether a row was actually there, so calling it
/// on an account with no picture answers 200 rather than inventing a 404 for a state that is already
/// what the caller asked for. `avatar_url` comes back empty so the caller can repaint from the
/// response instead of re-reading `/auth/me`.
///
/// TWO rows carry the picture and both must be cleared in one transaction: the BYTES in
/// `user_avatars` (what `get_avatar` streams) and the POINTER on `users.avatar_url` (what `/auth/me`
/// hands the console, and what the profile screen keys the "Remove picture" control off). The upload
/// writes both; a delete that only removed the bytes left `/auth/me` still returning the URL, so the
/// console kept painting a picture that 404s — measured live before this transaction was added.
pub async fn delete_avatar(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user ID".into()))?;

    let mut tx = state.pool.begin().await?;
    let removed = sqlx::query("DELETE FROM user_avatars WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("UPDATE users SET avatar_url = NULL, updated_at = NOW() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(
        json!({"status": "ok", "removed": removed, "avatar_url": ""}),
    ))
}

pub async fn change_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if req.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "New password must be at least 8 characters".into(),
        ));
    }

    let user_id = uuid::Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user ID".into()))?;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1 AND is_active = true")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    // Verify current password
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    // A corrupt stored hash cannot verify ANY current password, so this is the ordinary
    // "current password is incorrect" refusal — never a 500 naming the internal reason.
    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|_| AppError::Unauthorized("Current password is incorrect".into()))?;
    Argon2::default()
        .verify_password(req.current_password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Current password is incorrect".into()))?;

    // Hash new password
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let new_hash = argon2
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?
        .to_string();

    // SESSION REVOCATION (kanban t_80787a0f): a password change also ends every JWT minted
    // BEFORE it (`users.sessions_valid_from`), refused at the boundary read. Measured before this:
    // the same token kept answering 200 afterwards.
    sqlx::query(
        "UPDATE users SET password_hash = $1, sessions_valid_from = NOW(), updated_at = NOW() \
           WHERE id = $2",
    )
    .bind(&new_hash)
    .bind(user.id)
    .execute(&state.pool)
    .await?;

    Ok(Json(
        serde_json::json!({"message": "Password updated successfully"}),
    ))
}

pub async fn forgot_password(
    State(state): State<AppState>,
    body: Result<Json<ForgotPasswordRequest>, axum::extract::rejection::JsonRejection>,
) -> AppResult<Json<serde_json::Value>> {
    let Json(req) = body.map_err(crate::error::json_rejection)?;
    if let Some(user) = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE lower(email) = $1 AND is_active = true",
    )
    .bind(crate::security::email_addr::lookup_key(&req.email))
    .fetch_optional(&state.pool)
    .await?
    {
        // tenants.status enforcement, part 2 of 4 (kanban t_af890bbf): a retired workspace cannot
        // obtain NEW credentials either. The response body below is byte-identical to the one at
        // the end of this function, so this adds no account-existence signal — the mail just is
        // not sent, and the user learns the workspace is retired at login.
        if !crate::handlers::tenant_handler::workspace_access_allowed(
            &state.pool,
            user.tenant_id,
            &user.role,
        )
        .await?
        {
            return Ok(Json(serde_json::json!({
                "message": "If the email exists, a password reset link has been sent"
            })));
        }

        let token = uuid::Uuid::new_v4().to_string();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);

        sqlx::query("UPDATE password_resets SET used = true WHERE user_id = $1 AND used = false")
            .bind(user.id)
            .execute(&state.pool)
            .await
            .ok();

        sqlx::query("INSERT INTO password_resets (user_id, token, expires_at) VALUES ($1, $2, $3)")
            .bind(user.id)
            .bind(&token)
            .bind(expires_at)
            .execute(&state.pool)
            .await?;

        // A reserved / harness destination can only bounce (kanban t_e4d94bd9): Mailgun counts the
        // bounce against the sending domain's reputation, and the fleet's own probes call THIS route
        // with addresses like `probe-prof-…@example.invalid`. Withhold exactly what the signup path
        // withholds — the reset row is still written, only the mail that cannot arrive is skipped. A
        // real customer's address is unaffected.
        match crate::security::probe_addr::harness_domain(&user.email) {
            Some(domain) => {
                tracing::info!(
                    email = %user.email,
                    domain = %domain,
                    "forgot-password: reset email SUPPRESSED — reserved/harness address; a send could only bounce"
                );
                // Record the withheld attempt: without it the operator's "last email" line kept
                // showing an earlier, unrelated send, which reads as "the reset went out".
                crate::email::record_withheld(
                    &state.pool,
                    "password reset email",
                    &user.email,
                    &format!("'{domain}' is a fleet-harness / reserved address"),
                )
                .await;
            }
            None => {
                match send_reset_email(&state.pool, user.tenant_id, &user.email, &token, &user.name)
                    .await
                {
                    Ok(_) => tracing::info!("Password reset email sent to {}", user.email),
                    Err(e) => tracing::error!(
                        "Failed to send password reset email to {}: {}",
                        user.email,
                        e
                    ),
                }
            }
        }
        // Send password reset email via SMTP
    }

    Ok(Json(
        serde_json::json!({"message": "If the email exists, a password reset link has been sent"}),
    ))
}

pub async fn reset_password(
    State(state): State<AppState>,
    body: Result<Json<ResetPasswordRequest>, axum::extract::rejection::JsonRejection>,
) -> AppResult<Json<serde_json::Value>> {
    let Json(req) = body.map_err(crate::error::json_rejection)?;
    if req.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "New password must be at least 8 characters".into(),
        ));
    }

    let reset = sqlx::query(
        "SELECT id, user_id, expires_at FROM password_resets WHERE token = $1 AND used = false AND expires_at > NOW()",
    )
    .bind(&req.token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".into()))?;

    use sqlx::Row;
    let reset_id: uuid::Uuid = reset.get("id");
    let user_id: uuid::Uuid = reset.get("user_id");

    // tenants.status enforcement, part 3 of 4 (kanban t_af890bbf): completing a reset would write
    // a new credential into a retired workspace. Refused BEFORE the UPDATE below, so the one-shot
    // token is not consumed by a refusal.
    let owner = sqlx::query_as::<_, (uuid::Uuid, String)>(
        "SELECT tenant_id, role FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".into()))?;
    if !crate::handlers::tenant_handler::workspace_access_allowed(&state.pool, owner.0, &owner.1)
        .await?
    {
        return Err(AppError::Forbidden(
            crate::auth::global_auth::WORKSPACE_INACTIVE.into(),
        ));
    }

    // Hash new password
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let new_hash = argon2
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?;

    // SESSION REVOCATION (kanban t_80787a0f): a password change also ends every JWT minted
    // BEFORE it (`users.sessions_valid_from`), refused at the boundary read. Measured before this:
    // the same token kept answering 200 afterwards.
    sqlx::query(
        "UPDATE users SET password_hash = $1, sessions_valid_from = NOW(), updated_at = NOW() \
           WHERE id = $2",
    )
    .bind(new_hash.to_string())
    .bind(user_id)
    .execute(&state.pool)
    .await?;

    sqlx::query("UPDATE password_resets SET used = true WHERE id = $1")
        .bind(reset_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(
        serde_json::json!({"message": "Password has been reset successfully"}),
    ))
}

pub async fn get_usage(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let tenant_id: uuid::Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;
    let usage = features::get_usage_json(&state, tenant_id).await;
    Ok(Json(usage))
}

// ── CHANGE OF SIGN-IN ADDRESS (cron 2026-10-18) ────────────────────────────────────────────────
//
// The profile screen told the customer "Your sign-in address cannot be changed here" and no route
// anywhere in the app could change `users.email`. Signup collects NAME + EMAIL only, so ONE typo
// produced an account that could never receive another message from us, with no way out.
//
// The change is PROVEN, not merely requested: the link goes to the NEW address, and only that
// mailbox can complete it. `users.email` moves at confirmation time, never at request time — so a
// mistyped new address cannot lock the customer out of their own account.
//
// A reserved/harness address (example.invalid and friends) is WITHHELD at source, exactly like
// signup and password reset: the pending row is still written so the flow stays exercisable, but no
// provider request is made and the panel records a `withheld` non-send.

#[derive(Debug, Deserialize)]
pub struct RequestEmailChangeRequest {
    pub new_email: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmEmailChangeRequest {
    pub token: String,
}

pub async fn request_email_change(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<RequestEmailChangeRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // Same single validator every other writer of `users.email` uses.
    let new_email = crate::security::email_addr::normalize(&req.new_email)
        .map_err(AppError::UnprocessableEntity)?;
    let user_id = Uuid::parse_str(&auth.user_id)
        .map_err(|_| AppError::Unauthorized("Invalid user ID".into()))?;
    let tenant_id: Uuid = auth
        .tenant_id
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid tenant".into()))?;

    let current = sqlx::query_as::<_, (String, String)>(
        "SELECT email, name FROM users WHERE id = $1 AND is_active = true",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    if current.0.eq_ignore_ascii_case(&new_email) {
        return Err(AppError::BadRequest(
            "That is already your sign-in address".into(),
        ));
    }

    // Uniqueness is checked here so a clash reads as a sentence, not a 500 from the column.
    let taken: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE lower(email) = $1 AND is_active = true AND id <> $2",
    )
    .bind(&new_email)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    if taken.is_some() {
        return Err(AppError::Conflict(
            "That address is already used by another account".into(),
        ));
    }

    // NOTHING IS WRITTEN FOR AN ADDRESS THAT CANNOT BE MAILED (measured live 2026-10-26).
    // This handler used to insert the pending row FIRST and notice the reserved/harness class
    // afterwards, so a reserved address (a probe, or a typo into `example.invalid`) RETIRED the
    // customer's genuine pending change (`used = true`) and left a live, 2-hour row nobody could
    // ever confirm — while the response said "Nothing has been changed". The sentence was right
    // and the code was wrong. Check the address BEFORE the retire/insert: same answer, no writes,
    // and the real request the customer already had stays confirmable.
    if let Some(domain) = crate::security::probe_addr::harness_domain(&new_email) {
        tracing::info!(
            email = %new_email,
            domain = %domain,
            "email-change: reserved/harness address refused before any write; nothing sent, nothing stored"
        );
        crate::email::record_withheld(
            &state.pool,
            "sign-in address change",
            &new_email,
            &format!("'{domain}' is a fleet-harness / reserved address"),
        )
        .await;
        return Ok(Json(json!({
            "status": "pending",
            "delivered": false,
            "new_email": new_email,
            "message": "This address is a reserved test address, so no mail was sent. Nothing has been changed."
        })));
    }

    let token = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + chrono::Duration::hours(2);

    // One live request per user: retire the previous one before inserting.
    sqlx::query("UPDATE email_changes SET used = true WHERE user_id = $1 AND used = false")
        .bind(user_id)
        .execute(&state.pool)
        .await
        .ok();

    sqlx::query(
        "INSERT INTO email_changes (user_id, new_email, token, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(&new_email)
    .bind(&token)
    .bind(expires_at)
    .execute(&state.pool)
    .await?;

    let link = format!("https://app.funnelswift.net/confirm-email-change.html?token={token}");

    // Every recipient that reaches this point is a real address: the reserved/harness class was
    // refused ABOVE, before the pending row existed, so this is the only send path left.
    {
        let mut vars: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        vars.insert("name", current.1.as_str());
        vars.insert("new_email", new_email.as_str());
        vars.insert("link", link.as_str());
        vars.insert("token", token.as_str());
        if let Err(e) = crate::email::send_template_email_for_tenant(
            &state.pool,
            tenant_id,
            tenant_id,
            &new_email,
            "email_change",
            &vars,
        )
        .await
        {
            tracing::error!(
                "Failed to send sign-in address confirmation to {}: {}",
                new_email,
                e
            );
            // No link reached the customer, so the pending row must not look usable.
            sqlx::query("UPDATE email_changes SET used = true WHERE token = $1")
                .bind(&token)
                .execute(&state.pool)
                .await
                .ok();
            return Err(AppError::UpstreamRefused(
                "We could not send the confirmation link, so nothing was changed. Try again in a moment."
                    .into(),
            ));
        }
    }

    Ok(Json(json!({
        "status": "pending",
        "delivered": true,
        "new_email": new_email,
        "message": "Check the new address for a confirmation link. Your sign-in email stays the same until you open it.",
    })))
}

/// `POST /api/v1/auth/email-change/confirm` — anonymous by necessity: the token IS the credential
/// the customer just received, and a link opened in a browser carries no session.
pub async fn confirm_email_change(
    State(state): State<AppState>,
    Json(req): Json<ConfirmEmailChangeRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let token = req.token.trim().to_string();
    if token.is_empty() {
        return Err(AppError::BadRequest(
            "The confirmation link is missing its code".into(),
        ));
    }

    let row = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT id, user_id, new_email FROM email_changes
         WHERE token = $1 AND used = false AND expires_at > NOW()",
    )
    .bind(&token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| {
        AppError::BadRequest("That confirmation link is invalid or has expired".into())
    })?;

    // The address may have been taken between request and confirmation.
    let taken: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE lower(email) = $1 AND is_active = true AND id <> $2",
    )
    .bind(&row.2)
    .bind(row.1)
    .fetch_optional(&state.pool)
    .await?;
    if taken.is_some() {
        return Err(AppError::Conflict(
            "That address has since been used by another account, so nothing was changed".into(),
        ));
    }

    let user = sqlx::query_as::<_, (Uuid, String)>(
        "UPDATE users SET email = $1, email_verified = true, updated_at = NOW()
         WHERE id = $2 AND is_active = true
         RETURNING tenant_id, role",
    )
    .bind(&row.2)
    .bind(row.1)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::BadRequest("That account is no longer active".into()))?;

    // tenants.status enforcement: a retired workspace cannot take a new credential either.
    if !crate::handlers::tenant_handler::workspace_access_allowed(&state.pool, user.0, &user.1)
        .await?
    {
        return Err(AppError::Forbidden(
            crate::auth::global_auth::WORKSPACE_INACTIVE.into(),
        ));
    }

    sqlx::query("UPDATE email_changes SET used = true WHERE id = $1")
        .bind(row.0)
        .execute(&state.pool)
        .await?;

    Ok(Json(json!({
        "status": "ok",
        "email": row.2,
        "message": "Your sign-in address has been changed. Use it the next time you sign in.",
    })))
}
