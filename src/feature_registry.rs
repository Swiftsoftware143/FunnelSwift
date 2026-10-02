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
        key: "max_api_keys",
        label: "API keys",
        kind: Kind::Limit,
        unit: "keys",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/api-keys",
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
        key: "max_routing_targets",
        label: "Routing targets",
        kind: Kind::Limit,
        unit: "targets",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/integration-targets",
        read_by_gate: true,
    },
    Spec {
        key: "max_integrations",
        label: "Integration targets",
        kind: Kind::Limit,
        unit: "targets",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/integration-targets",
        read_by_gate: true,
    },
    Spec {
        key: "max_affiliates",
        label: "Affiliate accounts",
        kind: Kind::Limit,
        unit: "accounts",
        jsonb_key: None,
        column: None,
        enforced_by: "POST /api/v1/affiliates",
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
        key: "has_api",
        label: "API access",
        kind: Kind::Boolean,
        unit: "",
        jsonb_key: Some("api_access"),
        column: Some("has_api"),
        enforced_by: "POST /api/v1/api-keys",
        read_by_gate: true,
    },
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
        enforced_by: "POST /api/v1/integration-targets",
        read_by_gate: true,
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

pub fn spec(key: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.key == key)
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
        Some("has_api") => "UPDATE plans SET has_api = $2 WHERE id = $1",
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
}
