# AGENTS.md — Vibe Engineering Rules for AI Agents

## Rust Guardrails (MANDATORY)
- **Zero unsafe blocks** unless explicitly approved by the Lead Architect
- **Zero .unwrap() or .expect()** in non-test production code — use `thiserror`/`anyhow`
- **All async state must implement Send + Sync**
- **Parameterized SQL only** — use `sqlx::query_as!` for compile-time validation
- **Secrets in env vars only** — never hardcoded
- **cargo fmt** before commit

## Verification Sequence (NON-NEGOTIABLE)
After ANY code change:
1. `cargo check` — syntax + borrow checker. Read stderr. Fix. Repeat until clean.
2. `cargo test` — all tests must pass
3. `cargo clippy -- -D warnings` — zero warnings tolerated
4. `cargo fmt -- --check` — formatting must be consistent

## Self-Correction Loop
- Compiler error → read diagnostic → understand → fix → re-compile
- Test failure → fix logic → re-run
- Clippy warning → clean up → re-run
- **NEVER paste errors to a human. FIX THEM.**
- 3 attempts max, then escalate with evidence of what you tried.

## Hermes Delegation Pattern
For complex feature implementation:
1. Draft trait signatures and types FIRST
2. Run `cargo check` to validate types before writing method bodies
3. Then implement method logic — iterate with check/test/clippy
4. Re-run full verification before declaring done

## Build Lock Protocol
- ALWAYS use `/opt/swift/build-lock.sh <app> <command>`
- Never raw `cargo build --release` on shared repos
- Exit 2 = another bot building → wait 30s, retry once
- Stale lock >30min: clear and proceed

## Post-Deploy Smoke Test
- `curl -s -o /dev/null -w "%{http_code}" <domain>` must return 200

## Project File Architecture
```
src/api_router.rs
src/auth/handlers.rs
src/auth/middleware.rs
src/auth/mod.rs
src/auth/models.rs
src/config.rs
src/db.rs
src/email.rs
src/error.rs
src/features.rs
src/handlers/adaswift_provision.rs
src/handlers/admin_handler.rs
src/handlers/affiliate_handler.rs
src/handlers/affiliate_lead_handler.rs
src/handlers/affiliate_payout_handler.rs
src/handlers/affiliate_portal_handler.rs
src/handlers/affiliate_product_handler.rs
src/handlers/affiliate_tracking_handler.rs
src/handlers/api_key_handler.rs
src/handlers/bulk_handler.rs
src/handlers/campaigns_handler.rs
src/handlers/checkout_handler.rs
src/handlers/coreswift_push.rs
src/handlers/cross_app_webhook_handler.rs
src/handlers/dashboard_handler.rs
src/handlers/email_template_handler.rs
src/handlers/funnel_handler.rs
src/handlers/incentiveswift_handler.rs
src/handlers/insight_handler.rs
src/handlers/integration_target_handler.rs
src/handlers/kinetic_handler.rs
src/handlers/lead_handler.rs
src/handlers/linkedin.rs
src/handlers/linkedin_auth_handler.rs
src/handlers/mod.rs
src/handlers/ocr.rs
src/handlers/plan_handler.rs
src/handlers/plan_tag_handler.rs
src/handlers/portfolio_handler.rs
src/handlers/portfolio_sync_handler.rs
```
