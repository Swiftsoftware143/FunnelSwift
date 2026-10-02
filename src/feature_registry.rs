//! The plan feature registry — ONE list of every key this app's gates read.
//!
//! WHY IT EXISTS: the plan rows and the gate code are written at different times, so the key the
//! gate reads and the key the data carries drift apart silently (kanban t_35acff73, AF-5). With
//! ONE registry the admin panel, the `grant-top-tier` action and the gates themselves cannot
//! disagree about what "the top tier gets everything" means — and the drift test at the bottom of
//! this file turns a new gate key that nobody registered into a red build.
//!
//! ABSENCE RULES (they are NOT the same for the two kinds, and that is the whole trap):
//!   * Boolean flag — absent jsonb key falls back to the `has_*` column, whose NOT NULL DEFAULT
//!     is `false`, so ABSENCE MEANS REFUSED. A plan that configures nothing refuses.
//!   * Numeric limit — no `feature_limits` row and a NULL column means "not configured", which
//!     `enforce_feature_limit` treats as ALLOW. Absence therefore makes a limit INERT (it
//!     enforces nothing) rather than denying; that is exactly what the top tier looked like.

use crate::error::AppResult;
use crate::features;
use crate::state::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Boolean,
    Limit,
}

/// One gated plan key, as the gates and the admin panel see it.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Canonical key: what `enforce_feature_flag` / `enforce_feature_limit` is called with, and
    /// what `PUT /api/v1/admin/plans/entitlement` takes back from the panel.
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    /// Human unit for a limit ("cards", "leads", …); empty for a boolean.
    pub unit: &'static str,
    /// Boolean: the `plans.features` key the flag is written to and read from first.
    pub jsonb_key: Option<&'static str>,
    /// Boolean: the `has_*` column kept in step with the jsonb key.
    /// Limit: the `plans` column the gate falls back to when `feature_limits` has no row.
    pub column: Option<&'static str>,
    /// Where the key is enforced, for the operator reading the panel.
    pub enforced_by: &'static str,
    /// True when one of the two *scanned* gate helpers reads this key. The drift test below
    /// asserts both directions, so this cannot quietly rot.
    pub read_by_gate: bool,
}

/// Every key a gate in this app can read. Order is the panel's render order.
pub const SPECS: &[Spec] = &[
    // ── numeric limits ─────────────────────────────────────────────────────────────────────
    Spec {
        key: "max_cards",
        label: "Card limit",
        kind: Kind::Limit,
        unit: "cards",
        jsonb_key: None,
        column: Some("max_cards"),
        enforced_by: "POST /api/v1/kinetic/cards",
        read_by_gate: true,
    },
    Spec {
        key: "max_leads",
        label: "Lead limit",
        kind: Kind::Limit,
        unit: "leads",
        jsonb_key: None,
        column: Some("max_leads"),
        enforced_by: "POST /api/v1/leads",
        read_by_gate: true,
    },
    Spec {
        key: "max_tags",
        label: "Tag limit",
        kind: Kind::Limit,
        unit: "tags",
        jsonb_key: None,
        column: Some("max_tags"),
        enforced_by: "POST /api/v1/tags",
        read_by_gate: true,
    },
    Spec {
        key: "max_forms",
        label: "Web-to-lead forms",
        kind: Kind::Limit,
        unit: "forms",
        jsonb_key: None,
        column: Some("max_forms"),
        enforced_by: "POST /api/v1/web-to-lead",
        read_by_gate: true,
    },
    Spec {
        key: "max_custom_domains",
        label: "Custom domains",
        kind: Kind::Limit,
        unit: "domains",
        jsonb_key: None,
        column: Some("max_custom_domains"),
        enforced_by: "PUT /api/v1/kinetic/custom-domain",
        read_by_gate: true,
    },
    Spec {
        key: "max_qr_codes",
        label: "QR codes",
        kind: Kind::Limit,
        unit: "codes",
        jsonb_key: None,
        column: Some("max_qr_codes"),
        enforced_by: "POST /api/v1/kinetic/qr",
        read_by_gate: true,
    },
    Spec {
        key: "max_ocr_scans",
        label: "Card scans",
        kind: Kind::Limit,
        unit: "scans",
        jsonb_key: None,
        column: Some("max_ocr_scans"),
        enforced_by: "POST /api/v1/ocr/parse-card",
        read_by_gate: true,
    },
    Spec {
        key: "max_action_buttons",
        label: "Action buttons per card",
        kind: Kind::Limit,
        unit: "buttons",
        jsonb_key: None,
        column: Some("max_action_buttons"),
        enforced_by: "POST /api/v1/kinetic/cards/:id/buttons",
        read_by_gate: false,
    },
    Spec {
        key: "max_webhooks",
        label: "Webhooks",
        kind: Kind::Limit,
        unit: "webhooks",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/webhooks",
        read_by_gate: true,
    },
    Spec {
        key: "max_portfolios",
        label: "Portfolio companies",
        kind: Kind::Limit,
        unit: "companies",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/portfolio-companies",
        read_by_gate: true,
    },
    Spec {
        key: "max_tag_groups",
        label: "Tag groups",
        kind: Kind::Limit,
        unit: "groups",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/tag-groups",
        read_by_gate: true,
    },
    Spec {
        key: "max_integrations",
        label: "Integration Center connections",
        kind: Kind::Limit,
        unit: "connections",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/provider-keys",
        read_by_gate: true,
    },
    Spec {
        key: "max_affiliates",
        label: "Affiliate accounts",
        kind: Kind::Limit,
        unit: "accounts",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/affiliates, POST /api/v1/affiliate/signup",
        read_by_gate: true,
    },
    Spec {
        key: "max_team_members",
        label: "Team members",
        kind: Kind::Limit,
        unit: "members",
        jsonb_key: None,
        column: Some("max_team_members"),
        enforced_by: "(dashboard usage counter only — no gate calls it)",
        read_by_gate: false,
    },
    // ── boolean flags ──────────────────────────────────────────────────────────────────────
    Spec {
        key: "has_webhooks",
        label: "Webhooks",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("webhooks"),
        column: Some("has_webhooks"),
        enforced_by: "POST /api/v1/webhooks",
        read_by_gate: true,
    },
    Spec {
        key: "has_import_export",
        label: "Lead export",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("import_export"),
        column: Some("has_import_export"),
        enforced_by: "GET /api/v1/leads/export",
        read_by_gate: true,
    },
    Spec {
        key: "has_dual_routing",
        label: "Dual routing",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("dual_routing"),
        column: Some("has_dual_routing"),
        // A capability marker on the plan cards (suite / agency), NOT a gate: the delivery path it
        // used to switch on was the retired `target_software` writer, and multi-destination delivery
        // is what the Webhooks engine does for every subscribed webhook — gated by `has_webhooks`.
        // Removing the key from `plans.features` would be a pricing change, so it stays declared.
        enforced_by:
            "(no separate gate — the Webhooks engine, POST /api/v1/webhooks, gated by has_webhooks)",
        read_by_gate: false,
    },
    Spec {
        key: "has_mini_funnels",
        label: "Mini funnels",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("mini_funnels"),
        column: Some("has_mini_funnels"),
        enforced_by: "POST /api/v1/kinetic/cards (mini_funnel type)",
        read_by_gate: true,
    },
    Spec {
        key: "has_card_gating",
        label: "Card gating (password / consent / age)",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("card_gating"),
        column: Some("has_card_gating"),
        enforced_by: "PUT /api/v1/kinetic/cards/:id/password|gating",
        read_by_gate: true,
    },
    Spec {
        key: "has_analytics",
        label: "Card analytics",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("analytics"),
        column: Some("has_analytics"),
        enforced_by: "GET /api/v1/card-analytics/:card_id",
        read_by_gate: true,
    },
    Spec {
        key: "premium_themes",
        label: "Premium themes & templates",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("premium_themes"),
        column: None,
        enforced_by: "POST /api/v1/kinetic/cards, GET /api/v1/kinetic/themes",
        read_by_gate: false,
    },
    Spec {
        key: "has_white_label",
        label: "White label",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("white_label"),
        column: Some("has_white_label"),
        enforced_by: "(card render SQL — branding)",
        read_by_gate: false,
    },
    Spec {
        key: "has_remove_branding",
        label: "Remove branding",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("remove_branding"),
        column: Some("has_remove_branding"),
        enforced_by: "(card render SQL — branding)",
        read_by_gate: false,
    },
    Spec {
        key: "has_multi_tenant",
        label: "Multi-tenant workspaces",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("multi_tenant"),
        column: Some("has_multi_tenant"),
        enforced_by: "(no gate reads it yet)",
        read_by_gate: false,
    },
];

/// Keys RETIRED as plan residue — no gate in this crate reads them any more. Two waves so far:
///
/// * `migrations/086_retire_ungated_feature_limit_keys.sql` (kanban t_090f3e00) — live
///   `feature_limits` rows that NO gate read. Each one was either a SECOND NAME for a capability a
///   `SPECS` key already enforces, or named a quantity this app never measures (or measures but has
///   no route that writes the counted row). A second one followed the same day when the resource it
///   counted was retired (088, kanban t_0aaf0bc5).
/// * `migrations/090_retire_api_key_plan_residue.sql` (kanban t_726416be) — the two remaining keys
///   of the deleted api-key credential surface: the `max_api_keys` limit (rows deleted) and the
///   `has_api` flag. Decision t_538505de ARM b retired the surface; t_5a3c2d9c deleted its routes,
///   handler, model and signup mint, which left both keys pointing at `POST /api/v1/api-keys`
///   (404). A key whose only route is gone is residue, so BOTH leave `SPECS` here and the console
///   can no longer offer a control or a limit for a capability that does not exist.
///
/// Scope note (the difference between the two waves): the `feature_limits` ROWS and the registry
/// entries are retired here; the `plans.has_api` COLUMN values and the `features->>'api_access'`
/// jsonb keys on capture-starter / suite / agency are plan/billing DATA and are LEFT AS THEY ARE —
/// removing what a plan sells is a pricing change (kanban t_0aaf0bc5 made the same call for
/// `has_dual_routing`), not a residue fix. That open question is recorded on t_726416be.
///
/// The rows are GONE, and this list is what keeps the retirement honest:
///   * `GET /api/v1/admin/plans/registry` publishes it as `retired_keys`, so the operator's console
///     can say "retired, enforces nothing" instead of leaving a blank where a number used to be;
///   * the drift test below fails the build if one of them is ever re-registered as a gate key —
///     two vocabularies for one entitlement is the defect this list exists to prevent.
///
/// The authored numbers live in the migration headers, the admin guide's "Retired plan keys"
/// section and /opt/swift/audits/t_09{{0f3e00,0aaf0bc5,726416be}}/ — deliberately not as live rows.
pub const RETIRED_KEYS: &[(&str, &str)] = &[
    (
        "max_api_keys",
        "the credential surface it capped was retired (decision t_538505de ARM b; routes, handler, \
         model and signup mint deleted by t_5a3c2d9c), so its only gate, POST /api/v1/api-keys, \
         answers 404. The two live rows (agency -1, kinetic-pro -1) were deleted by migration 090 — \
         and the authored values are a plan-page number nothing could ever honour: no api key \
         exists to count. Not to be re-registered without a real mint route and a key selector",
    ),
    (
        "has_api",
        "the boolean half of the same retirement: POST /api/v1/api-keys is deleted, so no gate reads \
         the flag and the admin plan matrix can no longer advertise it as a capability. The \
         `plans.has_api` column and the `features->>'api_access'` jsonb keys on the three paid plans \
         are deliberately LEFT IN PLACE as plan/billing DATA — whether a paid plan still sells \
         \"API access\" is a pricing call (kanban t_726416be), not this registry's",
    ),
    (
        "kinetic_themes",
        "premium theme access is the `premium_themes` boolean (enforce_theme_access / \
         enforce_template_access); a COUNT of premium themes is a second vocabulary nothing reads",
    ),
    (
        "max_api_calls",
        "no request meter exists in the crate — no call/usage table and no per-period accounting, and \
         nothing increments a counter (kanban t_1d600303, DECIDED 2026-10-02: API-call volume stays \
         UNMETERED on every plan; the rows stay deleted and this reason is the record). A cap needs a \
         counter AND a price — enforcing 1000/10000 calls per period against live tenants is the \
         owner's pricing call — so a request meter belongs to the API-surface question already carded \
         for him (t_f2457903, ARM B), not to this registry. Do not re-register it: with no counter the \
         gate would read 0 and the cap could never fire (the `max_ocr_scans` trap)",
    ),
    (
        "max_plans",
        "no tenant-scoped quantity named plan: `plans` is the global price list (no tenant_id); \
         both authored values were -1, so a wire would have been a no-op",
    ),
    (
        "max_portfolio_companies",
        "same COUNT(portfolio_companies) as `max_portfolios`, which is the key the gate is called \
         with (both names were inserted in the same 2026-07-06 wave)",
    ),
    (
        "max_routing_rules",
        "no entity called a routing rule exists in this crate or the served consoles; \
         target_software rows are already counted by `max_routing_targets` and `max_integrations`",
    ),
    (
        "max_routing_targets",
        "its only mechanism was the `target_software` resource (migration 088 drops it); the \
         authored values are byte-identical to the surviving `max_webhooks` on the same plans \
         (kinetic-free 5/5, kinetic-pro -1/-1) and agency carries -1 with no `max_webhooks` row, \
         where absence means not-configured = allow, so the retire loses no sold allowance",
    ),
    (
        "max_settings",
        "COUNT(tenant_settings) is internal key/value storage, not a plan entitlement; both \
         authored values were -1",
    ),
    (
        "max_target_software",
        "a THIRD name for COUNT(target_software) after `max_routing_targets` and `max_integrations`",
    ),
    (
        "storage_mb",
        "nothing in this crate STORES bytes, so there is nothing to measure: the live schema has no \
         file/blob/upload table (the ONLY binary column in the whole database is \
         _sqlx_migrations.checksum) and no code computes a size (no octet_length / \
         pg_total_relation_size / SUM(size)); the logo and avatar fields hold LINKS, not uploads. \
         DECIDED 2026-10-02 (kanban t_5ce5b5a3): storage stays UNMETERED on every plan — measured, \
         the largest live workspace holds ~39 kB of rows and the ENTIRE database (16 workspaces) is \
         14 MB, so the authored 50/500 MB caps sat 1,000x above anything stored and a wire would \
         have been a knob that can never fire. Do not re-register it: a storage cap needs a real \
         store (a byte-bearing table plus an upload route) before any limit has a quantity to \
         compare against",
    ),
    (
        "team_members",
        "the second spelling of `max_team_members`; and no route adds a second member to an \
         EXISTING tenant, so the cap could not fire whichever name it used",
    ),
];

pub fn spec(key: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.key == key)
}

/// `(key, reason)` pairs for the admin console — the retired vocabulary, published as
/// `retired_keys` by `GET /api/v1/admin/plans/registry`.
pub fn retired_keys() -> Vec<(String, String)> {
    RETIRED_KEYS
        .iter()
        .map(|(k, why)| ((*k).to_string(), (*why).to_string()))
        .collect()
}

/// The plan that IS the top tier. FunnelSwift's `plans` table has no `sort_order` or `is_active`
/// column, so the live ordering is price (highest first), then name for determinism.
pub async fn top_plan_id(state: &AppState) -> AppResult<Option<Uuid>> {
    Ok(
        sqlx::query_scalar("SELECT id FROM plans ORDER BY price DESC NULLS LAST, name LIMIT 1")
            .fetch_optional(&state.pool)
            .await?,
    )
}

/// Every plan as its own JSON row (`to_jsonb`), so a resolver can read any column by name
/// without building SQL from it. Ordered top tier first.
pub async fn plan_rows(state: &AppState) -> AppResult<Vec<Value>> {
    Ok(sqlx::query_scalar::<_, Value>(
        "SELECT to_jsonb(p) FROM plans p ORDER BY p.price DESC NULLS LAST, p.name",
    )
    .fetch_all(&state.pool)
    .await?)
}

/// `(plan_id, feature_key, limit_value)` for every `feature_limits` row — the store the panel
/// writes custom per-plan limits into and the store the limit gate reads FIRST.
pub async fn feature_limit_rows(state: &AppState) -> AppResult<Vec<(Uuid, String, i32)>> {
    let rows = sqlx::query("SELECT plan_id, feature_key, limit_value FROM feature_limits")
        .fetch_all(&state.pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<Uuid, _>("plan_id"),
                r.get::<String, _>("feature_key"),
                r.get::<Option<i32>, _>("limit_value").unwrap_or(0),
            )
        })
        .collect())
}

/// Resolved value of `spec` for one plan row, under the SAME rules the gates use.
/// * boolean -> `true`/`false`
/// * limit -> the number, or `null` when neither a `feature_limits` row nor a `plans` column
///   carries the key ("not configured" — which the gate treats as allow)
pub fn resolved_value(spec: &Spec, row: &Value, feature_limit: Option<i32>) -> Value {
    match spec.kind {
        Kind::Boolean => Value::Bool(features::plan_row_flag(row, spec.key).unwrap_or(false)),
        Kind::Limit => match feature_limit {
            Some(v) => json!(v),
            None => spec
                .column
                .and_then(|c| row.get(c))
                .and_then(|v| v.as_i64())
                .map(|n| json!(n))
                .unwrap_or(Value::Null),
        },
    }
}

/// True when the resolved value enforces nothing: a limit with no row and no column, which the
/// house absence rule treats as ALLOW. A boolean is never inert — absence refuses.
pub fn is_inert(spec: &Spec, value: &Value) -> bool {
    spec.kind == Kind::Limit && value.is_null()
}

/// How generous a resolved value is, under the gate's own semantics. `None` = refused/disabled.
/// Used to assert the superset property ("no other plan beats the top tier on any key").
/// * boolean: `true` = granted; `false` = 402
/// * limit: absent (`null`) = NOT CONFIGURED = the gate allows, so it ranks as unlimited;
///   `-1` = unlimited; `0` = disabled; `n` = a cap of n
pub fn generosity(spec: &Spec, value: &Value) -> Option<i64> {
    match spec.kind {
        Kind::Boolean => match value.as_bool() {
            Some(true) => Some(i64::MAX),
            _ => None,
        },
        Kind::Limit => match value.as_i64() {
            None => Some(i64::MAX),
            Some(-1) => Some(i64::MAX),
            Some(0) => None,
            Some(n) => Some(n),
        },
    }
}

/// Write a boolean grant into BOTH stores the app reads: the `features` jsonb key (the gate's
/// first choice) and the legacy `has_*` column (which the card renderer reads with raw SQL).
/// Writing only one is how the "panel says no / the gate says yes" class of drift starts.
pub async fn write_boolean(
    state: &AppState,
    plan_id: Uuid,
    spec: &Spec,
    on: bool,
) -> AppResult<()> {
    let alias = spec.jsonb_key.unwrap_or(spec.key);
    sqlx::query(
        "UPDATE plans SET features = COALESCE(features,'{}'::jsonb) || jsonb_build_object($2::text, $3::boolean), updated_at = NOW() WHERE id = $1",
    )
    .bind(plan_id)
    .bind(alias)
    .bind(on)
    .execute(&state.pool)
    .await?;

    let q = match spec.column {
        // `has_api` was removed here with the key itself (kanban t_726416be): the flag is no longer
        // in SPECS, so no `spec.column` can be `has_api`, and the registry's own entitlement route
        // refuses the retired key before it reaches this function.
        Some("has_webhooks") => "UPDATE plans SET has_webhooks = $2 WHERE id = $1",
        Some("has_dual_routing") => "UPDATE plans SET has_dual_routing = $2 WHERE id = $1",
        Some("has_mini_funnels") => "UPDATE plans SET has_mini_funnels = $2 WHERE id = $1",
        Some("has_card_gating") => "UPDATE plans SET has_card_gating = $2 WHERE id = $1",
        Some("has_remove_branding") => "UPDATE plans SET has_remove_branding = $2 WHERE id = $1",
        Some("has_white_label") => "UPDATE plans SET has_white_label = $2 WHERE id = $1",
        Some("has_multi_tenant") => "UPDATE plans SET has_multi_tenant = $2 WHERE id = $1",
        Some("has_analytics") => "UPDATE plans SET has_analytics = $2 WHERE id = $1",
        Some("has_import_export") => "UPDATE plans SET has_import_export = $2 WHERE id = $1",
        // `premium_themes` is jsonb-only; nothing to keep in step.
        _ => return Ok(()),
    };
    sqlx::query(q)
        .bind(plan_id)
        .bind(on)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// Write a numeric grant: `Some(v)` upserts the plan's `feature_limits` row for the key (the
/// store the limit gate reads first), `None` deletes it so the key falls back to the plan column.
pub async fn write_limit(
    state: &AppState,
    plan_id: Uuid,
    key: &str,
    value: Option<i32>,
) -> AppResult<()> {
    match value {
        None => {
            sqlx::query("DELETE FROM feature_limits WHERE plan_id = $1 AND feature_key = $2")
                .bind(plan_id)
                .bind(key)
                .execute(&state.pool)
                .await?;
        }
        Some(v) => {
            let updated = sqlx::query(
                "UPDATE feature_limits SET limit_value = $3 WHERE plan_id = $1 AND feature_key = $2",
            )
            .bind(plan_id)
            .bind(key)
            .bind(v)
            .execute(&state.pool)
            .await?
            .rows_affected();
            if updated == 0 {
                sqlx::query(
                    "INSERT INTO feature_limits (id, plan_id, feature_key, limit_value) VALUES (gen_random_uuid(), $1, $2, $3)",
                )
                .bind(plan_id)
                .bind(key)
                .bind(v)
                .execute(&state.pool)
                .await?;
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Drift gate: a gate key that is not in this registry (or a registry entry claiming to be read
// by a gate that no longer calls it) fails `cargo test` instead of silently enforcing nothing.
// ─────────────────────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// The literal a gate call passes as its key: the first `"…"` after the call name, which
    /// also covers calls whose arguments wrap onto following lines.
    fn first_literal(s: &str) -> Option<String> {
        let start = s.find('"')? + 1;
        let rest = &s[start..];
        let end = rest.find('"')?;
        let k = &rest[..end];
        if k.is_empty() || k.len() > 40 || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        Some(k.to_string())
    }

    fn called_keys() -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let is_rs = p.extension().and_then(|e| e.to_str()) == Some("rs");
                if !is_rs || p.file_name().and_then(|n| n.to_str()) == Some("feature_registry.rs") {
                    continue;
                }
                let src = fs::read_to_string(&p).unwrap_or_default();
                for call in ["enforce_feature_limit(", "enforce_feature_flag("] {
                    let mut idx = 0;
                    while let Some(rel) = src[idx..].find(call) {
                        let abs = idx + rel;
                        idx = abs + call.len();
                        let line_start = src[..abs].rfind('\n').map(|x| x + 1).unwrap_or(0);
                        let line_end = src[abs..].find('\n').map(|x| abs + x).unwrap_or(src.len());
                        // skip the definition itself
                        if src[line_start..line_end].contains("fn ") {
                            continue;
                        }
                        let start = abs + call.len();
                        let mut end = (start + 300).min(src.len());
                        while end < src.len() && !src.is_char_boundary(end) {
                            end += 1; // em dash / box-drawing chars in comments are multi-byte
                        }
                        if let Some(k) = first_literal(&src[start..end]) {
                            out.push(k);
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn every_gate_key_is_registered_and_every_registered_gate_key_is_called() {
        let called = called_keys();
        assert!(
            !called.is_empty(),
            "scan found no gate call sites — the scanner is broken"
        );
        for k in &called {
            assert!(
                spec(k).is_some(),
                "gate calls enforce_* with \"{k}\" but it is not in feature_registry::SPECS"
            );
        }
        for s in SPECS.iter().filter(|s| s.read_by_gate) {
            assert!(
                called.iter().any(|k| k == s.key),
                "feature_registry says \"{}\" is read by a gate, but no enforce_* call site uses it",
                s.key
            );
        }
    }

    /// The 9 keys retired by migration 086 (kanban t_090f3e00) must never come back as gate keys:
    /// re-registering one would re-create the two-vocabularies-for-one-entitlement defect, and the
    /// first test would then happily accept a call site using it. Every entry also has to carry the
    /// measured reason it was retired — a bare name is not a decision.
    #[test]
    fn retired_keys_are_never_registered_as_gate_keys() {
        assert!(!RETIRED_KEYS.is_empty(), "the retirement list is empty");
        let mut seen = std::collections::BTreeSet::new();
        for &(k, reason) in RETIRED_KEYS {
            assert!(
                !reason.trim().is_empty(),
                "retired key \"{k}\" carries no recorded reason"
            );
            assert!(
                spec(k).is_none(),
                "retired key \"{k}\" is back in feature_registry::SPECS — one vocabulary per capability"
            );
            assert!(seen.insert(k), "retired key \"{k}\" is listed twice");
        }
        // The published shape the admin endpoint hands the console must stay in step with the list.
        let published = retired_keys();
        assert_eq!(published.len(), RETIRED_KEYS.len());
        for (i, (k, why)) in published.iter().enumerate() {
            assert_eq!(k, RETIRED_KEYS[i].0);
            assert_eq!(why, RETIRED_KEYS[i].1);
        }
    }
}
