//! `credential_watch` — tell the OPERATOR when a customer's first password never reached them.
//!
//! WHY THIS EXISTS (measured live 2026-11-15 on the sibling app ADASwift): a real customer signed
//! up on 2026-11-13, the mail carrying their first password never left the box, and the account sat
//! unusable for two days. Every piece of the state existed — the signup door latches
//! `users.password_undelivered` when the mail does not go, the Tenants screen shows a badge on that
//! row, and the console has a door that sets a first password — but a badge is only read by somebody
//! who happens to open that screen. NOBODY WAS EVER TOLD.
//!
//! FunnelSwift mailed the operator for a new workspace, a plan request and a support request, and had
//! no arm at all for the one event that leaves a customer unable to use the product they just bought.
//! This worker closes that gap: it looks, on a timer, for a latch that is set and unreported, and
//! mails the operator once per account.
//!
//! Two rules decide the whole design, both learned the hard way:
//!
//! * **"Already reported" means the operator was actually MAILED**, not that a row exists. A send the
//!   transport withholds (a harness address, an unconfigured provider) writes a row too, and treating
//!   that as a report retires the account for ever — exactly how the ADASwift customer became
//!   unreportable. So the dedupe names the operator address AND requires status `sent`, and a pass
//!   whose own row comes back `withheld` leaves the account unreported so the next pass retries.
//! * **Machinery must never ring a human.** A harness signup ALWAYS lands with the latch set — its
//!   credentials mail is suppressed by construction — so without a gate this watch would mail the
//!   operator once per probe signup. The gate is on the REQUESTER (the customer identity and the
//!   workspace name), never on a route: the same door serves real customers.

use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// How often the watch looks. Fifteen minutes bounds how long a locked-out customer waits for a
/// human to be told, and costs one cheap indexed read when there is nothing to report.
pub const WATCH_INTERVAL_SECS: u64 = 15 * 60;

/// Grace period. A signup whose mail is still in flight, or a mail layer that is momentarily slow,
/// must not page the operator for a customer who is about to receive their password anyway.
pub const MIN_AGE_MINUTES: i64 = 30;

/// How far back the watch looks. A latch set years ago belongs to an account the operator has
/// already dealt with by other means; two weeks is the window in which the arrival is still news.
pub const LOOKBACK_DAYS: i64 = 14;

/// Accounts per pass, so one pass never sends an unbounded burst of mail.
pub const BATCH: i64 = 5;

/// Boot-time worker, started from `main` next to the probe sweeper and the webhook retry worker.
/// A failure inside one pass is logged and the loop continues — an alerter must never take the app
/// down.
pub fn spawn_credential_watch(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(WATCH_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // The first tick fires immediately, so a deploy reports the existing backlog instead of
            // waiting a quarter of an hour.
            ticker.tick().await;
            match alert_once(&pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(reported = n, "undelivered-credential alert"),
                Err(e) => tracing::warn!(error = %e, "undelivered-credential watch failed"),
            }
        }
    });
}

/// Report at most [`BATCH`] unreported accounts whose first password never reached them.
///
/// Returns how many alerts actually reached the operator. A withheld send does not count, so the
/// number is "a human was told", not "a function ran".
pub async fn alert_once(pool: &PgPool) -> Result<usize, String> {
    // WHO the alert goes to is resolved BEFORE the selector, because "already reported" has to mean
    // "delivered to THIS operator". No configured address means nothing to send to, and this code
    // never invents a mailbox.
    let settings = crate::platform_settings::load(pool).await;
    let Some(to) = settings
        .operator_alert_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return Ok(0);
    };

    let rows: Vec<(Uuid, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT u.id,
               u.email,
               COALESCE(NULLIF(btrim(t.name), ''), u.email),
               COALESCE(NULLIF(btrim(u.name), ''), ''),
               u.created_at::date::text
          FROM users u
          JOIN tenants t ON t.id = u.tenant_id
         WHERE u.password_undelivered
           AND u.is_active
           AND u.closed_at IS NULL
           AND u.created_at < now() - make_interval(mins => $1::int)
           AND u.created_at > now() - make_interval(days => $2::int)
           -- already reported: a row that names this account AND actually reached the operator we
           -- are about to mail. A withheld row (harness recipient, unconfigured provider) is not a
           -- report, and neither is a row addressed to anyone else.
           AND NOT EXISTS (
                 SELECT 1 FROM email_send_log l
                  WHERE l.subject LIKE '%' || u.id::text || '%'
                    AND l.to_email = $4
                    AND l.status = 'sent')
         ORDER BY u.created_at ASC
         LIMIT $3
        "#,
    )
    .bind(MIN_AGE_MINUTES)
    .bind(LOOKBACK_DAYS)
    .bind(BATCH)
    .bind(&to)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("select undelivered credentials: {e}"))?;

    if rows.is_empty() {
        return Ok(0);
    }

    let mut reported = 0usize;
    for (user_id, customer_email, workspace, owner_name, signed_up) in rows {
        // A probe signup always carries the latch (its mail cannot be delivered by construction), so
        // this gate is what stops the watch mailing a human once per harness run. Requester, not
        // route: the same door serves real customers. Both the customer identity and the workspace
        // name are judged, because a harness workspace can carry a clean-looking address.
        let synthetic =
            crate::security::probe_addr::synthetic_requester(&customer_email, &owner_name)
                .or_else(|| crate::security::probe_addr::synthetic_requester("", &workspace));
        if let Some(reason) = synthetic {
            tracing::info!(
                user = %user_id,
                reason = %reason,
                "undelivered-credential alert WITHHELD — the account is a synthetic identity"
            );
            continue;
        }

        match crate::email::send_undelivered_credential_operator_alert(
            pool,
            user_id,
            &customer_email,
            &workspace,
            &owner_name,
            &signed_up,
            &to,
        )
        .await
        {
            Ok(status) if status == "withheld" => {
                // `deliver` answers Ok for a recipient the transport withholds. The account stays
                // UNREPORTED so a later pass retries once the operator address is a real mailbox,
                // instead of being silently retired for ever.
                tracing::warn!(
                    user = %user_id,
                    to = %to,
                    "undelivered-credential alert WITHHELD — the operator address cannot receive \
                     mail, so this account stays unreported and the next pass retries"
                );
            }
            Ok(_) => {
                tracing::info!(user = %user_id, "reported an undelivered credential to the operator");
                reported += 1;
            }
            Err(e) => {
                tracing::warn!(user = %user_id, error = %e, "undelivered-credential alert failed");
            }
        }
    }
    Ok(reported)
}
