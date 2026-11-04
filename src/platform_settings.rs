//! Platform-wide settings — the operator controls that apply to the WHOLE installation.
//!
//! Why this module exists (measured 2026-11-04). The admin console's **System Settings** screen
//! used to render a "General Settings" card with a default-plan picker, a per-user tenant limit and
//! a registration-open toggle, and **not one of them was wired**: no reader anywhere in the console,
//! no handler in `src/`, and `/api/v1/settings` is this app's TENANT `{key,value}` store, not a
//! platform config. A previous pass removed the inert controls rather than fake them and said so on
//! the screen ("need a backend before they can be set from here"). This module is that backend, for
//! the two controls that have a real meaning in THIS schema.
//!
//! * `registration_open` — enforced at BOTH signup doors ([`crate::handlers::public_signup_handler`]
//!   and the register door). Closed means a new account cannot be created at all; existing accounts
//!   are untouched, so closing the door never locks anyone out.
//! * `default_plan_slug` — the product side a brand-new workspace starts on when the caller named no
//!   plan of its own. It must name an ACTIVE, price-0 plan: a paid plan can never be a default seat,
//!   because nothing has been charged and the account would be given a paid tier for free.
//!
//! A third control the old card carried — "max tenants per user" — is deliberately NOT shipped: in
//! this schema one `users` row belongs to exactly ONE `tenants` row (`users.tenant_id`), so a
//! per-user workspace cap has no value it could ever admit or refuse. Shipping a box that cannot
//! change anything is the same defect the inert card was.
//!
//! Storage: ONE row in `system_settings` (`key` is UNIQUE, `value` is jsonb), the table this install
//! already uses for cross-app configuration. One key, one object, one writer — so the read is a
//! single indexed lookup and a partial write cannot leave the pair inconsistent.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// The one `system_settings.key` this app owns.
pub const SETTINGS_KEY: &str = "funnelswift_platform_settings";

/// The platform's own defaults, byte-identical to the behaviour this app had before the control
/// existed: signups open, and no plan override (the signup page's own `source`/`plan` decides).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformSettings {
    #[serde(default = "default_registration_open")]
    pub registration_open: bool,
    #[serde(default)]
    pub default_plan_slug: Option<String>,
}

fn default_registration_open() -> bool {
    true
}

impl Default for PlatformSettings {
    fn default() -> Self {
        Self {
            registration_open: true,
            default_plan_slug: None,
        }
    }
}

/// Read the settings. AN ABSENT ROW IS NOT AN ERROR — a fresh install has never saved anything, and
/// answering `Default::default()` keeps every behaviour exactly as it shipped.
///
/// A DATABASE ERROR also answers the default, and that is deliberate rather than lazy: the only
/// caller that reads this on a customer path is the signup door, and failing that door with a 500
/// because a settings table is unreadable would stop every new customer while telling them nothing.
/// The read one screen away (`GET /api/v1/admin/platform-settings`) surfaces the real error to the
/// operator, which is where it can be acted on.
pub async fn load(pool: &PgPool) -> PlatformSettings {
    let row: Result<Option<Value>, sqlx::Error> =
        sqlx::query_scalar("SELECT value FROM system_settings WHERE key = $1")
            .bind(SETTINGS_KEY)
            .fetch_optional(pool)
            .await;

    match row {
        Ok(Some(v)) => serde_json::from_value::<PlatformSettings>(v).unwrap_or_else(|e| {
            tracing::error!(
                error = %e,
                key = SETTINGS_KEY,
                "platform settings: the stored row is not a PlatformSettings object — using the \
                 shipped defaults. Re-save the screen to rewrite it."
            );
            PlatformSettings::default()
        }),
        Ok(None) => PlatformSettings::default(),
        Err(e) => {
            tracing::error!(
                error = %e,
                key = SETTINGS_KEY,
                "platform settings: read failed — using the shipped defaults (signups stay open)"
            );
            PlatformSettings::default()
        }
    }
}

/// Write the settings in one statement, so the pair can never be half-saved.
pub async fn save(pool: &PgPool, s: &PlatformSettings) -> Result<(), sqlx::Error> {
    let value = serde_json::to_value(s).unwrap_or_else(|_| json!({}));
    sqlx::query(
        "INSERT INTO system_settings (id, key, value) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(Uuid::new_v4())
    .bind(SETTINGS_KEY)
    .bind(&value)
    .execute(pool)
    .await?;
    Ok(())
}

/// The plans an operator may pick as the default seat: price-0 tiers only.
///
/// Returned as `(slug, name, side)`; the side is what the signup door actually applies, because the
/// resolver seats the free tier of a side.
pub async fn seatable_plans(pool: &PgPool) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    sqlx::query_as(
        // `plans` on this app carries NO `active` column (measured 2026-11-04 from
        // `information_schema.columns`); a retired tier is deleted by `plan_handler`, not flagged.
        // So the only predicate is PRICE — the same `price = 0` test `plan_resolver` uses.
        "SELECT slug, name, side::text FROM plans
          WHERE price = 0
          ORDER BY side, slug",
    )
    .fetch_all(pool)
    .await
}
