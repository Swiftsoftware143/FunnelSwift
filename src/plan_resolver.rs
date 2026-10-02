//! `plan_resolver` — which plan a NEW signup lands on, resolved from columns the `plans` table
//! GUARANTEES rather than from a slug literal (kanban t_3641f326).
//!
//! THE DEFECT THIS CLOSES, measured 2026-10-02 on live `funnelswift` vs a from-zero database
//! booted from the deployed artifact: the two installs hold DISJOINT plan-slug vocabularies.
//! Live ships capture-free / kinetic-free / capture-starter / kinetic-pro / suite / agency.
//! A fresh build ships 000001's generic free / starter / pro / enterprise. Neither install can be
//! assumed to carry the other's, yet both signup paths did
//!
//!     let plan_slug = "capture-free"            // and "kinetic-free"
//!     SELECT id FROM plans WHERE slug = $1 LIMIT 1
//!     if let Some(pid) = plan_id { INSERT INTO tenant_plan_subscriptions (…) }
//!
//! with no `else` arm and a `let _ = ` on the insert. Measured: `plans WHERE slug='capture-free'`
//! is 1 row on live and 0 on a from-zero build, so a fresh install answered a 201 having created a
//! PLANLESS tenant, in silence. The mirrors-image of the same root: `tag_logic`'s paid test was
//! `matches!(slug, "pro" | "enterprise")`, a vocabulary that only exists on a from-zero build.
//!
//! THE ARM, decided by measurement and not by preference (the card allowed either "declare the
//! slug vocabulary canonical and seed it, with the owner's sign-off" or "stop depending on a slug
//! existing and make the miss LOUD"): the CODE stops naming slugs. It binds the signup's own free
//! tier through columns that cannot be absent —
//!
//!   * `plans.price` is `double precision NOT NULL DEFAULT 0`
//!   * `plans.side`  is `character varying(20) NOT NULL DEFAULT 'main'`
//!
//! — so every install resolves, and WHICH plan it resolved is reported instead of swallowed. On
//! live that resolves the product's own free tier (capture-free for the capture side,
//! kinetic-free for the kinetic side) exactly as before. On a fresh install, where the operator
//! has not yet split the generic tiers into capture/kinetic, both paths land on the install's free
//! tier and the server logs a warning naming the fix. Seeding FunnelSwift's real slugs/prices is
//! NOT done here: pricing is the operator's (`plan_handler` is the panel writer), which is why
//! the card separated it from the seed work of t_6c44acb9.

use crate::error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;

/// The free tier of the product a signup belongs to, expressed as the `plans.side` values that
/// tier may carry. `side` is `NOT NULL DEFAULT 'main'`, so unlike a slug it is a property no
/// install can lack.
///
/// Measured `side` values — live: `capture-free`/`capture-starter` = `main`,
/// `kinetic-free`/`kinetic-pro` = `kinetic`, `suite`/`agency` = `both`; from-zero: all four
/// generic tiers = `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductSide {
    /// `POST /api/v1/auth/register` — the capture entry point.
    Capture,
    /// `POST /api/v1/auth/signup` — the Kinetic Cards entry point.
    Kinetic,
}

impl ProductSide {
    /// The word this side is reported by, in logs and in the signup response body.
    pub fn label(self) -> &'static str {
        match self {
            ProductSide::Capture => "capture",
            ProductSide::Kinetic => "kinetic",
        }
    }

    /// The `plans.side` values this product's free tier may carry, best match first.
    fn sides(self) -> [&'static str; 2] {
        match self {
            ProductSide::Capture => ["main", "both"],
            ProductSide::Kinetic => ["kinetic", "both"],
        }
    }
}

/// The plan a signup was put on. `exact` is false when this install carries no free plan for the
/// signup's own product and its generic free tier was used instead — reported, never hidden.
#[derive(Debug)]
pub struct FreePlanAssignment {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub exact: bool,
}

/// What `assign_free_plan` did, published verbatim in the two signup responses.
#[derive(Debug)]
pub enum PlanOutcome {
    Assigned(FreePlanAssignment),
    /// No price-0 plan exists anywhere in this install, so the tenant has NO subscription. This is
    /// the one branch that used to be silent (the old `if let` had no `else`, and the insert's
    /// result was discarded) — it now logs at ERROR and says so in the response.
    Missing,
}

impl PlanOutcome {
    /// The plan slug the caller should report, or `None` when nothing was attached.
    pub fn slug(&self) -> Option<&str> {
        match self {
            PlanOutcome::Assigned(a) => Some(a.slug.as_str()),
            PlanOutcome::Missing => None,
        }
    }

    /// The structured form the signup responses publish, so an operator can see from the wire
    /// which plan a brand-new account got and whether it is the product's own free tier.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            PlanOutcome::Assigned(a) => serde_json::json!({
                "assigned": true,
                "plan_id": a.id.to_string(),
                "plan_slug": a.slug,
                "plan_name": a.name,
                // false => this install has no free tier for the signup's product and its
                // generic free tier was used; the server log names the operator fix.
                "exact_product_free_tier": a.exact,
            }),
            PlanOutcome::Missing => serde_json::json!({
                "assigned": false,
                "warning": "This install has no price-0 plan, so the account was created with no \
                            plan subscription. Create a free plan in Admin > Plans.",
            }),
        }
    }
}

/// The free tier of the signup's own product. `price = 0` and `side` are both NOT NULL columns,
/// and the ORDER BY keeps the choice deterministic if an operator ever prices two tiers at zero.
const SQL_PRODUCT_FREE: &str = "SELECT id, slug, name FROM plans \
     WHERE price = 0 AND side = ANY($1::text[]) \
     ORDER BY price ASC, slug ASC LIMIT 1";

/// This install's free tier, whatever side it carries.
const SQL_ANY_FREE: &str = "SELECT id, slug, name FROM plans \
     WHERE price = 0 ORDER BY price ASC, slug ASC LIMIT 1";

/// Put a brand-new tenant on the free tier of `side`, and report what happened.
///
/// `route` is the signup route being served, and only ever appears in logs.
pub async fn assign_free_plan(
    pool: &PgPool,
    tenant_id: Uuid,
    side: ProductSide,
    route: &str,
) -> AppResult<PlanOutcome> {
    let wanted: Vec<String> = side.sides().iter().map(|s| s.to_string()).collect();

    let product_free: Option<(Uuid, String, String)> = sqlx::query_as(SQL_PRODUCT_FREE)
        .bind(&wanted)
        .fetch_optional(pool)
        .await?;

    let assignment = match product_free {
        Some((id, slug, name)) => FreePlanAssignment {
            id,
            slug,
            name,
            exact: true,
        },
        None => match sqlx::query_as::<_, (Uuid, String, String)>(SQL_ANY_FREE)
            .fetch_optional(pool)
            .await?
        {
            Some((id, slug, name)) => FreePlanAssignment {
                id,
                slug,
                name,
                exact: false,
            },
            None => {
                tracing::error!(
                    route = %route,
                    tenant = %tenant_id,
                    side = %side.label(),
                    "signup: THIS INSTALL HAS NO PRICE-0 PLAN — the new tenant was created with NO \
                     plan subscription. An operator must create a free plan in Admin > Plans."
                );
                return Ok(PlanOutcome::Missing);
            }
        },
    };

    if assignment.exact {
        tracing::info!(
            route = %route,
            tenant = %tenant_id,
            plan = %assignment.slug,
            plan_id = %assignment.id,
            "signup: free plan assigned"
        );
    } else {
        tracing::warn!(
            route = %route,
            tenant = %tenant_id,
            plan = %assignment.slug,
            plan_id = %assignment.id,
            side = %side.label(),
            "signup: this install has no free plan for this product's side — the install's free \
             tier was used instead. Create a price-0 plan with side 'main' (capture) or 'kinetic' \
             to make the assignment exact."
        );
    }

    // NOT best-effort any more: the old `let _ = sqlx::query(…).execute(…)` discarded the result,
    // so a failed insert looked exactly like the intended no-op and produced a planless tenant
    // behind a 201. The tenant exists by the time we get here, so a failure is a real error.
    sqlx::query(
        r#"INSERT INTO tenant_plan_subscriptions (id, tenant_id, plan_id, status, start_date)
           VALUES ($1, $2, $3, 'active', NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(assignment.id)
    .execute(pool)
    .await?;

    Ok(PlanOutcome::Assigned(assignment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_product_prefers_its_own_side_then_the_shared_one() {
        assert_eq!(ProductSide::Capture.sides(), ["main", "both"]);
        assert_eq!(ProductSide::Kinetic.sides(), ["kinetic", "both"]);
        assert_eq!(ProductSide::Capture.label(), "capture");
        assert_eq!(ProductSide::Kinetic.label(), "kinetic");
    }

    #[test]
    fn a_missing_plan_is_reported_and_has_no_slug() {
        let outcome = PlanOutcome::Missing;
        assert_eq!(outcome.slug(), None);
        let json = outcome.to_json();
        assert_eq!(json["assigned"], serde_json::json!(false));
        let warning = json["warning"].as_str().unwrap_or_default().to_string();
        assert!(warning.contains("Admin > Plans"));
    }
}
