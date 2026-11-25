// Cross-app lead push to WorkflowSwift — the ONE leg of the legacy "cross-app push" family
// (commit 650e91c, 28 handlers) that has a real counterpart.
//
// The HTTP leg is GONE. POST /api/v1/push/workflowswift was removed 2026-11-25 (kanban
// t_739722ac): it had no caller anywhere — the lead-create flow delivers through `deliver_lead`
// directly (src/handlers/lead_handler.rs), and no shipped SPA, template, script, fleet tool or
// n8n workflow ever posted to it. The delivery function below is the app's ONE WorkflowSwift
// writer and stays: it is what the lead flow uses. Handler imports were dropped with the route.
//
// That family fabricated success: every leg below answered a 200 with a `json!({...})` built from
// the request payload, importing only AuthUser/Json/Value, calling nothing. The rest of the family
// was removed on 2026-09-25 (kanban t_0a6a93f1) with the measured reason recorded here:
//   * POST /api/v1/push/workflowswift/user — WorkflowSwift has NO internal user-provision route to
//     call: t_79d7d1d2 deleted its dead `tag_provision_handler.rs` and left a standing note at
//     src/routes.rs forbidding a re-add without a caller; cross-app account minting belongs to the
//     CRM hub (CoreSwift-CRM owns a real /api/v1/internal/tag-provision). Same class and treatment
//     as /api/v1/push/coreswift/user (t_5a9e4eb7).
//   * POST /api/v1/push/workflowswift/tag — a tag-only push has no correct payload: WorkflowSwift
//     attaches a tag to an entity through /internal/tags/assign, which needs a WorkflowSwift
//     tenant_id + tag id + entity, and a FunnelSwift tag name is not a WorkflowSwift tag id. Same
//     class as /api/v1/push/coreswift/tag (t_ae84b186).
//   * GET  /api/v1/push/workflowswift/health — a hardcoded {"connected":true,...} constant with no
//     caller, and nothing truthful to probe: measured live 2026-09-25, WorkflowSwift answers 404 on
//     both /health and /api/health (:8085), unlike CoreSwift's /api/health (200) which is what keeps
//     the sibling /api/v1/push/coreswift/health a genuinely working probe.
//
// What survives is the leg whose payload WorkflowSwift really accepts, and it is now REAL:
//   POST {workflowswift_url}/api/v1/incoming   (header: X-Internal-Key = INTERNAL_SYNC_KEY)
// — "the single endpoint all Swift tools push to" (WorkflowSwift src/handlers/incoming_handler.rs,
// whose own docs name FunnelSwift as a caller). MissedCallRespondr ships the identical call from its
// own workflowswift_push.rs on every contact creation; this is the FunnelSwift half that was a stub.
//
// Honest outcome reporting. WorkflowSwift answers {"status":"accepted","matched":false,...} and
// writes NOTHING when no *active* WorkflowSwift workflow matches source 'funnelswift' — that is
// returned as `no_workflow`, never as a success. A refusal (its key guard) or an unreachable target
// is an error carrying the target's own HTTP status, not a 200 with a made-up body.

use serde_json::{json, Value};
use std::time::Duration;

/// The payload WorkflowSwift's /api/v1/incoming accepts from a sibling app.
/// Owned strings so it can be moved into a spawned best-effort push.
#[derive(Debug, Clone, Default)]
pub struct LeadPush {
    pub email: Option<String>,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub source_entry_id: Option<String>,
    /// WorkflowSwift matches an active workflow by source or by this slug; defaults to "funnelswift".
    pub campaign_slug: Option<String>,
    pub data: Value,
}

/// What one delivery attempt produced — a description of what WorkflowSwift actually did.
#[derive(Debug, Clone, PartialEq)]
pub enum PushOutcome {
    /// A workflow matched: WorkflowSwift wrote a workflow instance and started it.
    Pushed,
    /// The target accepted the lead but matched no workflow, so it wrote nothing there.
    NoWorkflow,
    /// Not configured, unreachable, or refused. Carries the target's own status when it answered.
    Failed {
        status: Option<u16>,
        message: String,
    },
}

fn split_name(full: &str) -> (String, String) {
    let mut it = full.splitn(2, ' ');
    (
        it.next().unwrap_or("").trim().to_string(),
        it.next().unwrap_or("").trim().to_string(),
    )
}

/// The single WorkflowSwift delivery, shared by the API leg and by the best-effort push that fires
/// when a lead is created — so a lead cannot be delivered on one path and dropped on the other.
pub async fn deliver_lead(base_url: &str, lead: &LeadPush) -> PushOutcome {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return PushOutcome::Failed {
            status: None,
            message: "WorkflowSwift is not connected — set WORKFLOWSWIFT_URL".to_string(),
        };
    }

    let (first_name, last_name) = split_name(lead.name.as_deref().unwrap_or(""));
    let slug = lead
        .campaign_slug
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "funnelswift".to_string());

    let payload = json!({
        "source": "funnelswift",
        "campaign_slug": slug,
        "contact": {
            "first_name": first_name,
            "last_name": last_name,
            "email": lead.email,
            "phone": lead.phone,
            "business_name": lead.company,
        },
        "data": lead.data,
        "source_entry_id": lead.source_entry_id,
    });

    let url = format!("{base}/api/v1/incoming");
    let mut req = reqwest::Client::new()
        .post(&url)
        .json(&payload)
        .timeout(Duration::from_secs(5));

    let internal_key = std::env::var("INTERNAL_SYNC_KEY").unwrap_or_default();
    if !internal_key.is_empty() {
        req = req.header("X-Internal-Key", internal_key);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect();
            if !status.is_success() {
                tracing::warn!("workflowswift push refused: {status} {body}");
                return PushOutcome::Failed {
                    status: Some(status.as_u16()),
                    message: format!("WorkflowSwift returned {status}: {body}"),
                };
            }
            let matched = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("matched").and_then(Value::as_bool))
                .unwrap_or(false);
            if matched {
                PushOutcome::Pushed
            } else {
                PushOutcome::NoWorkflow
            }
        }
        Err(e) => {
            tracing::warn!("workflowswift push failed: {e}");
            PushOutcome::Failed {
                status: None,
                message: format!("WorkflowSwift unreachable: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unconfigured_target_is_reported_not_faked() {
        let out = deliver_lead("", &LeadPush::default()).await;
        match out {
            PushOutcome::Failed { status, message } => {
                assert_eq!(status, None);
                assert!(message.contains("not connected"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unreachable_target_is_reported_not_faked() {
        // Port 1 is never listening: the transport error arm, deterministic.
        let out = deliver_lead("http://127.0.0.1:1", &LeadPush::default()).await;
        match out {
            PushOutcome::Failed { status, message } => {
                assert_eq!(status, None, "a transport failure has no upstream status");
                assert!(message.contains("unreachable"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn name_splits_on_the_first_space() {
        assert_eq!(
            split_name("Ada Lovelace King"),
            ("Ada".to_string(), "Lovelace King".to_string())
        );
        assert_eq!(split_name(""), ("".to_string(), "".to_string()));
    }
}
