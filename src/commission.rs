//! The ONE place a commission rate is decided.
//!
//! Until 2026-09-29 there was no such place. A rate could sit on a plan, on a product (twice), on an
//! affiliate, on a tier, on a link and inside a `plan_tag_mappings` jsonb blob, and **nothing decided
//! which one applied**. The only lookup that existed anywhere read the *plan* rate with a hardcoded
//! `unwrap_or(20.0)` (`handlers/affiliate_portal_handler.rs`), and the commission rows themselves were
//! written with `amount` hardcoded to `0` (`tag_logic.rs`). So a sale could not produce a correct
//! number even when every rate was filled in properly.
//!
//! # The rule
//!
//! Highest wins, first match returns:
//!
//! 1. `affiliates.override_commission_rate` — a deliberate, per-person override. David's words:
//!    *"override a particular affiliate a higher commission on top of whatever they're getting."*
//!    This is why it sits above everything: it is the one rate a human set on purpose for this person.
//! 2. `affiliate_product_groups.commission_rate` — one rate covering the products that were grouped as
//!    "the same" (*"do an overall by selecting which products are the same"*).
//! 3. `affiliate_products.default_commission_rate` — the product's own rate, the field both admin
//!    consoles edit.
//! 4. `affiliate_products.commission_rate` — the product's legacy rate column. Only reached when the
//!    field the UI writes is NULL, so a legacy row still pays something sensible instead of falling
//!    through to the global default.
//! 5. `affiliates.commission_rate` — the affiliate's own standing rate (what self-signup
//!    writes from their plan).
//! 6. `plans.commission_rate` — the plan's rate, when a product is tied to a plan.
//! 7. [`FALLBACK_RATE`] — the same 20% the old code hardcoded, so behaviour with an empty database is
//!    unchanged from before this module existed.
//!
//! Every outcome carries **which rule produced it** ([`ResolvedRate::source`]) and a sentence a human
//! can read ([`ResolvedRate::explanation`]). That is deliberate: a number nobody can explain is a
//! number nobody can trust, and "why is this affiliate on 25%?" has to be answerable after the fact.
//!
//! # Why every rate is read with `::float8`
//!
//! The columns are `numeric(5,2)`. This fleet has already been bitten by decoding a Postgres type into
//! a Rust type that does not match it, so each rate is cast in SQL rather than trusted to round-trip.

use sqlx::PgPool;
use uuid::Uuid;

/// Used only when nothing in the chain supplies a rate. Matches the value the pre-existing code
/// hardcoded, so an empty database behaves exactly as it did before this module existed.
pub const FALLBACK_RATE: f64 = 20.0;

/// The rate that applies to one (product, affiliate) pair, plus where it came from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedRate {
    /// The percentage. `12.5` means 12.5%.
    pub rate: f64,
    /// Machine-readable rule name — stable, for code and tests.
    pub source: &'static str,
    /// One sentence a human can read, e.g. "the affiliate's own override (25%)".
    pub explanation: String,
    pub product_id: Option<Uuid>,
    pub affiliate_id: Option<String>,
    /// Set when the winning rule was a product group, so the UI can link to it.
    pub group_id: Option<Uuid>,
    pub group_name: Option<String>,
    /// Every rule that was considered, in order, with the rate each would have given. This is what
    /// makes "which rule won and what did it beat?" answerable in the admin panel.
    pub considered: Vec<Considered>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Considered {
    pub source: &'static str,
    pub rate: Option<f64>,
    pub detail: Option<String>,
    pub won: bool,
}

/// Resolve the commission rate for a product / affiliate pair.
///
/// Both arguments are optional so a caller can resolve a product alone (to show a product's effective
/// rate in the admin list) or an affiliate alone. Passing neither is legal and returns
/// [`FALLBACK_RATE`].
pub async fn resolve(
    pool: &PgPool,
    product_id: Option<Uuid>,
    affiliate_id: Option<&str>,
) -> ResolvedRate {
    let mut considered: Vec<Considered> = Vec::new();
    let mut group_id: Option<Uuid> = None;
    let mut group_name: Option<String> = None;
    // The affiliate's standing rate, used lower down the chain. Collected in step 1 so the affiliate
    // row is read once, not twice.
    let mut affiliate_standing: Option<f64> = None;

    // ── 1. the affiliate's deliberate override ───────────────────────────────────────────────────
    if let Some(aid) = affiliate_id {
        let row: Option<(Option<f64>, Option<String>, Option<f64>)> = sqlx::query_as(
            "SELECT override_commission_rate::float8, override_note, commission_rate::float8 \
             FROM affiliates WHERE id = $1",
        )
        .bind(aid)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if let Some((rate, note, standing)) = row {
            affiliate_standing = standing;
            let won = rate.is_some();
            considered.push(Considered {
                source: "affiliate_override",
                rate,
                detail: note,
                won,
            });
            if let Some(r) = rate {
                return finish(
                    r,
                    "affiliate_override",
                    format!("this affiliate's personal override ({r}%)"),
                    product_id,
                    affiliate_id,
                    group_id,
                    group_name,
                    considered,
                );
            }
        }
    }

    // ── 2–4. the product's own rate, and the group that may override it ──────────────────────────
    let mut product_default: Option<f64> = None;
    let mut product_legacy: Option<f64> = None;
    let mut plan_id: Option<Uuid> = None;

    if let Some(pid) = product_id {
        let row: Option<(Option<Uuid>, Option<f64>, Option<f64>, Option<Uuid>)> = sqlx::query_as(
            "SELECT group_id, default_commission_rate::float8, commission_rate::float8, plan_id \
             FROM affiliate_products WHERE id = $1",
        )
        .bind(pid)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if let Some((gid, def, legacy, plan)) = row {
            group_id = gid;
            product_default = def;
            product_legacy = legacy;
            plan_id = plan;
        }

        if let Some(gid) = group_id {
            let grow: Option<(String, Option<f64>)> = sqlx::query_as(
                "SELECT name, commission_rate::float8 FROM affiliate_product_groups WHERE id = $1",
            )
            .bind(gid)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

            if let Some((gname, grate)) = grow {
                group_name = Some(gname.clone());
                let won = grate.is_some();
                considered.push(Considered {
                    source: "product_group",
                    rate: grate,
                    detail: Some(gname),
                    won,
                });
                if let Some(r) = grate {
                    return finish(
                        r,
                        "product_group",
                        format!("the rate set for its product group ({r}%)"),
                        product_id,
                        affiliate_id,
                        group_id,
                        group_name,
                        considered,
                    );
                }
            }
        }

        considered.push(Considered {
            source: "product",
            rate: product_default,
            detail: None,
            won: product_default.is_some(),
        });
        if let Some(r) = product_default {
            return finish(
                r,
                "product",
                format!("this product's own rate ({r}%)"),
                product_id,
                affiliate_id,
                group_id,
                group_name,
                considered,
            );
        }

        considered.push(Considered {
            source: "product_legacy",
            rate: product_legacy,
            detail: None,
            won: product_legacy.is_some(),
        });
        if let Some(r) = product_legacy {
            return finish(
                r,
                "product_legacy",
                format!("this product's older rate field ({r}%)"),
                product_id,
                affiliate_id,
                group_id,
                group_name,
                considered,
            );
        }
    }

    // ── 5. the affiliate's own standing rate ─────────────────────────────────────────────────────
    // `affiliates.commission_rate` is what the self-signup writes from the user's plan. It sits BELOW
    // the product and group rates on purpose: a rate set on a specific product is a more precise
    // statement of intent than a general per-person rate, and before this step the column was read by
    // nothing at all — an affiliate's contractual rate had no effect on any payout.
    if let Some(r) = affiliate_standing {
        considered.push(Considered {
            source: "affiliate_rate",
            rate: Some(r),
            detail: None,
            won: true,
        });
        return finish(
            r,
            "affiliate_rate",
            format!("this affiliate's standing rate ({r}%)"),
            product_id,
            affiliate_id,
            group_id,
            group_name,
            considered,
        );
    }

    // ── 6. the plan the product is tied to ───────────────────────────────────────────────────────
    if let Some(plan) = plan_id {
        let prow: Option<(String, Option<f64>)> =
            sqlx::query_as("SELECT name, commission_rate::float8 FROM plans WHERE id = $1")
                .bind(plan)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);

        if let Some((pname, prate)) = prow {
            considered.push(Considered {
                source: "plan",
                rate: prate,
                detail: Some(pname.clone()),
                won: prate.is_some(),
            });
            if let Some(r) = prate {
                return finish(
                    r,
                    "plan",
                    format!("the {pname} plan's rate ({r}%)"),
                    product_id,
                    affiliate_id,
                    group_id,
                    group_name,
                    considered,
                );
            }
        }
    }

    // ── 7. nothing anywhere ──────────────────────────────────────────────────────────────────────
    considered.push(Considered {
        source: "default",
        rate: Some(FALLBACK_RATE),
        detail: None,
        won: true,
    });
    finish(
        FALLBACK_RATE,
        "default",
        format!("no rate was set anywhere, so the default applies ({FALLBACK_RATE}%)"),
        product_id,
        affiliate_id,
        group_id,
        group_name,
        considered,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    rate: f64,
    source: &'static str,
    explanation: String,
    product_id: Option<Uuid>,
    affiliate_id: Option<&str>,
    group_id: Option<Uuid>,
    group_name: Option<String>,
    considered: Vec<Considered>,
) -> ResolvedRate {
    ResolvedRate {
        rate,
        source,
        explanation,
        product_id,
        affiliate_id: affiliate_id.map(str::to_string),
        group_id,
        group_name,
        considered,
    }
}

/// The commission owed on a sale: `base_amount * rate%`, rounded to cents.
///
/// Rounded with `round()` on a 100-multiplied value rather than left as a float, because this number
/// becomes money in `affiliate_commissions.amount` (`numeric(12,2)`) and a lingering
/// `12.344999999999999` is not an acceptable payout.
pub fn commission_for(base_amount: f64, rate_percent: f64) -> f64 {
    if !base_amount.is_finite() || !rate_percent.is_finite() {
        return 0.0;
    }
    ((base_amount * rate_percent / 100.0) * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commission_is_rounded_to_cents() {
        // 33.33 * 7.5% = 2.49975 -> 2.50, not 2.49975
        assert_eq!(commission_for(33.33, 7.5), 2.5);
        // 123.45 * 17.5% = 21.60375 -> 21.60
        assert_eq!(commission_for(123.45, 17.5), 21.6);
        // a rate of 0 earns nothing — it must not be mistaken for "unset"
        assert_eq!(commission_for(500.0, 0.0), 0.0);
        // 20% of 100 is exactly 20, not 19.999999
        assert_eq!(commission_for(100.0, 20.0), 20.0);
    }

    #[test]
    fn nonsense_inputs_earn_zero_rather_than_nan() {
        // NaN cast into numeric(12,2) would fail the insert; returning 0 keeps the write honest.
        assert_eq!(commission_for(f64::NAN, 10.0), 0.0);
        assert_eq!(commission_for(100.0, f64::INFINITY), 0.0);
        assert_eq!(commission_for(f64::NEG_INFINITY, 10.0), 0.0);
    }

    #[test]
    fn the_fallback_matches_the_value_the_old_code_hardcoded() {
        // Guards against someone "tidying" this constant and silently changing what every unrated
        // product pays.
        assert_eq!(FALLBACK_RATE, 20.0);
    }
}
