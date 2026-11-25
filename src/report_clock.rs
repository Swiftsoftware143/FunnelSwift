//! `report_clock` — the clock a workspace's reports are computed in.
//!
//! WHY THIS EXISTS. Every report query used to group by the DATABASE's calendar: `created_at::date`,
//! `date_trunc('day', created_at)`, `date_trunc('week', CURRENT_DATE)`. The database runs in UTC, so
//! for a US business "today" rolled over at 19:00 or 20:00 local, and a lead captured at 23:30 was
//! filed under TOMORROW. Measured live 2026-11-25 on FunnelSwift: a lead created at 23:30
//! America/New_York answered as `2026-11-24` on the Insights day-by-day table — one day late, for a
//! customer whose own clock said the 23rd.
//!
//! WHAT IT IS. The zone the operator's reports are read in. It comes from `REPORT_TZ` so the
//! install's clock is configuration, not a hard-coded guess; when the variable is absent or blank it
//! falls back to the fleet's home-market zone (`America/New_York`), which is where this install's
//! customers are. An unusable value never panics: PostgreSQL resolves the name, and `report_tz()`
//! only chooses the string.
//!
//! WHAT IT IS NOT. It is not the storage clock. Rows stay `timestamptz` in UTC; only the boundaries
//! a human reads (a day, a week, a month) are computed in the workspace's zone, which is the same
//! distinction every calendar UI makes.

use std::env;

/// The default when the install does not name one: the business's home market.
const DEFAULT_REPORT_TZ: &str = "America/New_York";

/// The zone reports are bucketed in. Read per call — it is an env lookup, and a query is already
/// doing I/O, so caching it would only add a way for a config change to go unnoticed.
pub fn report_tz() -> String {
    match env::var("REPORT_TZ") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => DEFAULT_REPORT_TZ.to_string(),
    }
}
